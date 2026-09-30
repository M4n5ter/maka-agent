/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

//! Real WS reconnection through the Agent, after a committed local effect.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_tungstenite::{accept_async, tungstenite::Message};

// A 1 ms retry-after keeps the real Agent/journal budget test fast, independent
// of network timers. Real socket reconstruction is covered below.
#[derive(Clone)]
struct Unavailable {
    attempts: Arc<std::sync::Mutex<(bool, Vec<bool>)>>,
    replay_safe: bool,
}
impl maka_plugins::model::ProviderAdapter for Unavailable {
    fn open(
        &self,
        _: maka_plugins::model::Lifetime,
        _: CancellationToken,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<Arc<dyn maka_plugins::model::Session>, ModelError>,
    > {
        let session = self.clone();
        Box::pin(async move { Ok(Arc::new(session) as Arc<dyn maka_plugins::model::Session>) })
    }
}
impl maka_plugins::model::Session for Unavailable {
    fn try_switch_fallback_transport(&self) -> bool {
        let mut state = self.attempts.lock().unwrap();
        if state.0 {
            return false;
        }
        assert!(self.replay_safe);
        assert_eq!(
            state.1.len(),
            10,
            "fallback must wait for all primary retries"
        );
        state.0 = true;
        true
    }
    fn stream(
        &self,
        _: maka_plugins::model::Request,
        _: maka_plugins::model::Context,
    ) -> futures_util::future::BoxFuture<'static, Result<(), ModelError>> {
        let mut state = self.attempts.lock().unwrap();
        let fallback = state.0;
        state.1.push(fallback);
        let error = maka_runtime::model::error::ProviderFailure::new(
            maka_runtime::model::error::ProviderFailureReason::Network,
            "transport unavailable",
            self.replay_safe,
            Some(1),
        );
        Box::pin(async move { Err(ModelError::Provider(error)) })
    }
}

#[tokio::test]
async fn fallback_waits_for_retry_budget_and_gets_one_bounded_budget_of_its_own() {
    use maka_plugins::{
        composition::Scope,
        contributions::{Catalog, Staged},
        fiber::Fiber,
        model::Adapter,
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        for replay_safe in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let log = Arc::new(
                EventLog::open(&directory.path().join("events.sqlite"))
                    .await
                    .unwrap(),
            );
            let catalog = Catalog::default();
            let fiber = Fiber::new("fixture", "fixture", Scope::Profile).unwrap();
            fiber.begin_loading().unwrap();
            fiber.ready().unwrap();
            fiber.publish().unwrap();
            let attempts = Arc::new(std::sync::Mutex::new((false, Vec::new())));
            let mut staged = Staged::default();
            staged
                .insert(
                    "chat-completions",
                    Adapter {
                        provider: Arc::new(Unavailable {
                            attempts: attempts.clone(),
                            replay_safe,
                        }),
                    },
                )
                .unwrap();
            let _registration = catalog.register(&fiber.context(), staged).unwrap();
            let worker = Engine::new(
                log.clone(),
                ModelExecutor::new(1, Duration::from_secs(5))
                    .unwrap()
                    .with_catalog(catalog),
                CodeExecutor::new(1, CellLimits::default()).unwrap(),
            );
            let result = worker
                .run(
                    input("http://unused.invalid/v1", "budget"),
                    CancellationToken::new(),
                )
                .await;
            assert!(matches!(
                result,
                Err(RunError::Model(ModelError::Provider(_)))
            ));
            worker.drain().await;
            let attempts = attempts.lock().unwrap().1.clone();
            let expected: Vec<_> = if replay_safe {
                [false; 10].into_iter().chain([true; 10]).collect()
            } else {
                vec![false]
            };
            assert_eq!(attempts, expected);
            let prefix = log.prefix(100, 128 * 1024).await.unwrap();
            let requests: Vec<_> = prefix
                .events
                .iter()
                .filter_map(|event| match &event.event.fact {
                    Fact::ModelRequested {
                        input_digest,
                        source_high_water,
                        ..
                    } => Some((input_digest, source_high_water)),
                    _ => None,
                })
                .collect();
            assert_eq!(requests.len(), expected.len());
            assert!(requests.iter().all(|request| request == &requests[0]));
        }
    })
    .await
    .expect("both budgets must exhaust without an unbounded fallback loop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_retry_uses_frozen_full_input_and_never_reexecutes_settled_tools() {
    use crate::support::agent_loop::{self, Effect};
    tokio::time::timeout(Duration::from_secs(20), async {
        for cancel in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(socket).await.unwrap();
                let initial: Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                let item = json!({"type":"function_call","id":"function-1","call_id":"call-1","name":"echo","arguments":"{\"value\":42}"});
                for event in [
                    json!({"type":"response.created","response":{"id":"first","model":"test"}}),
                    json!({"type":"response.output_item.done","output_index":0,"item":item}),
                    json!({"type":"response.completed","response":{"id":"first","output":[item]}}),
                ] { socket.send(Message::Text(event.to_string().into())).await.unwrap(); }
                let delta: Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(delta["previous_response_id"], "first");
                assert_eq!(delta["input"][0]["type"], "function_call_output");
                // A partial local tool intent is unaccepted. It must not be
                // executed or included in the recovered model request.
                for event in [
                    json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"partial","content":[]}}),
                    json!({"type":"response.output_text.delta","item_id":"partial","delta":"unaccepted partial text"}),
                    json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"function-2","call_id":"never-execute","name":"echo","arguments":""}}),
                ] { socket.send(Message::Text(event.to_string().into())).await.unwrap(); }
                socket.close(None).await.unwrap();
                drop(socket);
                if cancel {
                    assert!(tokio::time::timeout(Duration::from_millis(1300), listener.accept()).await.is_err());
                    return;
                }
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(socket).await.unwrap(); // Recovery must stay on WS.
                let retry: Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                assert!(retry.get("previous_response_id").is_none());
                let mut expected = initial["input"].as_array().unwrap().clone();
                let mut replayed_item = item;
                replayed_item.as_object_mut().unwrap().remove("id"); // A fresh socket cannot reuse connection-local item IDs.
                expected.push(replayed_item);
                expected.extend(delta["input"].as_array().unwrap().iter().cloned());
                assert_eq!(retry["input"], json!(expected));
                assert!(!retry.to_string().contains("unaccepted partial text"));
                assert!(!retry.to_string().contains("never-execute"));
                let complete = json!({"type":"response.completed","response":{"id":"final","output":[{"type":"message","id":"answer","content":[{"type":"output_text","text":"done"}]}]}});
                socket.send(Message::Text(complete.to_string().into())).await.unwrap();
            });
            let directory = tempfile::tempdir().unwrap();
            let log = Arc::new(EventLog::open(&directory.path().join("events.sqlite")).await.unwrap());
            let count = Arc::new(AtomicUsize::new(0));
            let effect = Arc::new(Effect { log:log.clone(), count:count.clone() });
            let mut input = agent_loop::input(&base, "websocket", effect);
            input.provider.kind = ProviderKind::OpenaiResponses;
            input.provider_options = json!({"openai":{"store":false}});
            let worker = engine(log.clone());
            let cancellation = CancellationToken::new();
            let mut commits = log.subscribe_commits();
            let stop = async {
                if cancel {
                    loop {
                        let prefix = log.prefix(100, 128*1024).await.unwrap();
                        if prefix.events.iter().any(|event| matches!(event.event.fact, Fact::ModelInterrupted {status:ModelInterruption::RetryableFailure,..})) { break; }
                        commits.changed().await.unwrap();
                    }
                    cancellation.cancel();
                }
            };
            let (result, ()) = tokio::join!(worker.run(input, cancellation.clone()), stop);
            if cancel { assert!(matches!(result,Err(RunError::Cancelled))); } else {result.unwrap();}
            worker.drain().await;
            server.await.unwrap();
            assert_eq!(count.load(Ordering::SeqCst),1,"the previously settled tool must never run again");
            let prefix=log.prefix(100,128*1024).await.unwrap();
            let requests:Vec<_>=prefix.events.iter().filter_map(|event| match &event.event.fact {Fact::ModelRequested {input_digest,source_high_water,..}=>Some((input_digest,source_high_water)),_=>None}).collect();
            assert_eq!(requests.len(),if cancel {2} else {3});
            if !cancel {assert_eq!(requests[1],requests[2],"physical retry retains the canonical cut and frozen request");}
            assert_eq!(prefix.events.iter().filter(|event|matches!(event.event.fact,Fact::ModelInterrupted {status:ModelInterruption::RetryableFailure,..})).count(),1);
            assert_eq!(prefix.events.iter().filter(|event|matches!(event.event.fact,Fact::ToolDispatched {..})).count(),1);
        }
    }).await.expect("WS recovery and cancellation must settle");
}
