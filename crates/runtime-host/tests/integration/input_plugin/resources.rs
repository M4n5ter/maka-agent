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

use super::*;
use input::resources;
use maka_plugins::{
    contributions::{Publisher, Registration},
    fiber::Context,
};
use std::sync::Mutex;

const SESSION: &str = "resource-steering";
const PREPARE: &str = "example.resources";

#[derive(Clone)]
struct ResourceInput {
    calls: Arc<AtomicUsize>,
    marker: &'static str,
}
impl input::Provider for ResourceInput {
    fn prepare(
        &self,
        mut request: input::Request,
        _: maka_plugins::filesystem::ReadDirectory,
    ) -> BoxFuture<'static, Result<input::Outcome, maka_plugins::Error>> {
        let provider = self.clone();
        Box::pin(async move {
            let Some(selections) = request.selections.get(PREPARE) else {
                return Ok(input::Outcome::Unchanged);
            };
            if selections.len() != 1 || !["first:v1", "second:v1"].contains(&selections[0].as_str())
            {
                return Err(maka_plugins::Error::Invalid(
                    "stale resource selector".into(),
                ));
            }
            provider.calls.fetch_add(1, Ordering::SeqCst);
            let original = request.content.text.clone();
            request.content.text = format!("{} prepared {}", provider.marker, original);
            Ok(input::Outcome::Ready {
                content: request.content,
                receipt: json!({"selector":selections[0],"original":original,"marker":provider.marker}),
                required_tools: Default::default(),
                basis: None,
            })
        })
    }
}
impl resources::Provider for ResourceInput {
    fn query(
        &self,
        _: resources::Query,
        context: resources::Context,
    ) -> BoxFuture<'static, Result<resources::Page, maka_plugins::Error>> {
        Box::pin(async move {
            assert_eq!(context.session_id, SESSION);
            Ok(resources::Page {
                items: ["first:v1", "second:v1"]
                    .map(|id| resources::Item {
                        id: id.into(),
                        title: format!("Record {id}"),
                        description: None,
                    })
                    .into(),
                next_cursor: None,
            })
        })
    }
    fn resolve(
        &self,
        request: resources::Resolve,
        context: resources::Context,
    ) -> BoxFuture<'static, Result<resources::Value, maka_plugins::Error>> {
        Box::pin(async move {
            assert_eq!(context.session_id, SESSION);
            let label = request.id.trim_end_matches(":v1").to_owned();
            Ok(resources::Value {
                selector: request.id,
                label: label.clone(),
                quote: Some(maka_runtime::input::QuoteRef {
                    text: format!("immutable {label} excerpt"),
                    label: Some(label),
                    source_turn_id: None,
                    source: None,
                }),
            })
        })
    }
}

#[derive(Clone)]
struct ResourcePlugin {
    input: ResourceInput,
    binding: Arc<Mutex<Option<(Publisher, Context)>>>,
}
impl ResourcePlugin {
    fn stage(
        staged: &mut Staged,
        owner: &Context,
        input: ResourceInput,
    ) -> Result<(), maka_plugins::Error> {
        let input = Arc::new(input);
        resources::stage(
            staged,
            owner,
            PREPARE,
            input.clone(),
            resources::Resources {
                descriptor: resources::Descriptor {
                    title: maka_plugins::terminal_ui::Text::plain("Records"),
                },
                provider: input,
            },
            None,
        )
    }
    fn replace(&self, calls: Arc<AtomicUsize>) -> Registration {
        let (publisher, owner) = self.binding.lock().unwrap().clone().unwrap();
        resources::withdraw(&publisher, &owner, &[PREPARE.into()]).unwrap();
        let mut staged = Staged::default();
        Self::stage(
            &mut staged,
            &owner,
            ResourceInput {
                calls,
                marker: "replacement",
            },
        )
        .unwrap();
        publisher.publish(staged).unwrap()
    }
}
impl Plugin for ResourcePlugin {
    fn activate(
        &self,
        context: PluginContext,
        _: Value,
    ) -> BoxFuture<'static, Result<Staged, String>> {
        *self.binding.lock().unwrap() = Some((context.contributions, context.lifecycle.clone()));
        let input = self.input.clone();
        Box::pin(async move {
            let mut staged = Staged::default();
            staged
                .insert(
                    "example.review",
                    session::SessionBehavior::new(Arc::new(Business::default())),
                )
                .map_err(|e| e.to_string())?;
            Self::stage(&mut staged, &context.lifecycle, input).map_err(|e| e.to_string())?;
            Ok(staged)
        })
    }
}

async fn resolved(peer: &mut Peer, row: &Value, id: &str) -> Value {
    let binding = json!({"packageId":row["packageId"],"method":row["method"],"sessionId":SESSION});
    let bound = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(bound["ok"], true, "{bound}");
    assert_eq!(bound["result"]["target"], row["target"]);
    let opened = peer
        .rpc("plugin.remote", json!({"kind":"open_document"}))
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let document = &opened["result"]["document"];
    let result = peer.rpc("plugin.remote", json!({"kind":"call","binding":binding,
        "target":row["target"],"document":document,"input":{"kind":"resolve","id":id,"locale":"en"}})).await;
    assert_eq!(result["ok"], true, "{result}");
    let closed = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"close_document","document":document}),
        )
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    let value = result["result"]["value"].clone();
    assert_eq!(value["kind"], "resolved", "{value}");
    value
}

fn submit(epoch: &Value, id: &str, placement: &str, resource: &Value) -> Value {
    json!({"originHostEpoch":epoch,"sessionId":SESSION,"messageId":id,
        "content":{"text":id,"quotes":[resource["quote"]]},"placement":placement,
        "inputSelections":{PREPARE:[resource["selector"]]},"inputSelectionSources":[resource["source"]]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resource_queue_edit_promotion_and_restart_preserve_canonical_sources_without_repreparation()
 {
    tokio::time::timeout(Duration::from_secs(45), scenario())
        .await
        .unwrap();
}
async fn scenario() {
    let fixture = ClientFixture::new("maka-resource-steering-");
    std::fs::write(
        fixture.workspace.join("boundary.txt"),
        "settled model boundary",
    )
    .unwrap();
    let (provider, mut requests) = Provider::controlled().await;
    let model = configure(&fixture, &provider.base_url).await;
    let original_calls = Arc::new(AtomicUsize::new(0));
    let replacement_calls = Arc::new(AtomicUsize::new(0));
    let mut accepted = None;
    let mut expected_sources = None;
    for reopened in [false, true] {
        let plugin = ResourcePlugin {
            input: ResourceInput {
                calls: if reopened {
                    replacement_calls.clone()
                } else {
                    original_calls.clone()
                },
                marker: if reopened {
                    "restarted replacement"
                } else {
                    "original"
                },
            },
            binding: Default::default(),
        };
        let mut plugins = setup(&Business::default());
        plugins.builtins.insert(
            "example".into(),
            Arc::new(Definition {
                id: "example".into(),
                revision: "binary".into(),
                dependencies: vec![],
                inject: vec![],
                plugin: Arc::new(plugin.clone()),
            }),
        );
        let host = Host::open_with_options(
            fixture.owner(),
            None,
            HostOptions {
                plugins,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        #[cfg(unix)]
        let endpoint = fixture.workspace.parent().unwrap().join("resources.sock");
        #[cfg(windows)]
        let endpoint =
            std::path::PathBuf::from(format!(r"\\.\pipe\maka-resources-{}", uuid::Uuid::new_v4()));
        let stop = CancellationToken::new();
        let cleanup = stop.clone().drop_guard();
        let server = tokio::spawn(
            LocalListener::bind(&endpoint)
                .unwrap()
                .serve(host.clone(), stop),
        );
        let (mut peer, hello) = Peer::handshake(host.clone(), "resource-steering").await;
        peer.wait_for_plugins().await;
        let mut replacement = None;
        if !reopened {
            let created = peer.rpc("session.create", json!({"sessionId":SESSION,
                "workspace":{"kind":"host_path","path":fixture.workspace},
                "modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model},
                "orchestrationMode":"example.review","sandboxMode":"read-only","approvalPolicy":{"kind":"never"}})).await;
            assert_eq!(created["ok"], true, "{created}");
            let directory = peer.rpc("plugin.platform.query", json!({"view":"input_resources","rootId":format!("session:{SESSION}"),"limit":32})).await;
            assert_eq!(directory["ok"], true, "{directory}");
            let row = directory["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["provider"] == PREPARE)
                .unwrap()
                .clone();
            assert_eq!(
                row["scopeId"], "profile",
                "Session discovery includes its original Profile provider"
            );
            let first = resolved(&mut peer, &row, "first:v1").await;
            let second = resolved(&mut peer, &row, "second:v1").await;
            assert_eq!(
                original_calls.load(Ordering::SeqCst),
                0,
                "discovery does not prepare input"
            );
            let started = peer.rpc("turn.start", json!({"sessionId":SESSION,"turnId":"turn","content":{"text":"start"},"maxSteps":3})).await;
            assert_eq!(started["ok"], true, "{started}");
            let request = requests.recv().await.unwrap();
            let original = submit(&hello["hostEpoch"], "first", "next_turn", &first);
            let first_result = peer.rpc("turn.message.submit", original.clone()).await;
            assert_eq!(
                first_result["result"]["disposition"], "followup",
                "{first_result}"
            );
            let second_result = peer
                .rpc(
                    "turn.message.submit",
                    submit(&hello["hostEpoch"], "second", "current_turn", &second),
                )
                .await;
            assert_eq!(
                second_result["result"]["disposition"], "steering",
                "{second_result}"
            );
            let edited = peer.rpc("queue.entry.update", json!({"originHostEpoch":hello["hostEpoch"],"sessionId":SESSION,
                "entryId":"first","updateId":"edit-first","expectedQueueRevision":second_result["result"]["queueRevision"],"text":"edited first"})).await;
            assert_eq!(edited["ok"], true, "{edited}");
            let promoted = peer
                .rpc(
                    "queue.entry.promote",
                    json!({"originHostEpoch":hello["hostEpoch"],"sessionId":SESSION,
                "entryId":"first","promoteId":"promote-first"}),
                )
                .await;
            assert_eq!(promoted["ok"], true, "{promoted}");
            assert_eq!(original_calls.load(Ordering::SeqCst), 3);
            replacement = Some(plugin.replace(replacement_calls.clone()));
            // A successful readonly tool call reaches another model boundary in this same Run.
            request.reply.send(json!({"index":0,"delta":{"tool_calls":[{"index":0,"id":"boundary","type":"function",
                "function":{"name":"Read","arguments":json!({"path":fixture.workspace.join("boundary.txt")}).to_string()}}]},"finish_reason":"tool_calls"})).unwrap();
            let next = requests.recv().await.unwrap();
            let text = next.body.to_string();
            assert!(text.contains("original prepared edited first"), "{text}");
            assert!(text.contains("original prepared second"), "{text}");
            assert!(!text.contains("replacement prepared"));
            next.reply
                .send(json!({"index":0,"delta":{"content":"done"},"finish_reason":"stop"}))
                .unwrap();
            loop {
                let turn = peer
                    .rpc("turn.query", json!({"sessionId":SESSION,"turnId":"turn"}))
                    .await;
                match turn["result"]["status"].as_str() {
                    Some("completed") => break,
                    Some("failed" | "cancelled") => panic!("{turn}"),
                    _ => tokio::task::yield_now().await,
                }
            }
            let sources = peer
                .rpc(
                    "session.sources.query",
                    json!({"sessionId":SESSION,"turnId":"turn"}),
                )
                .await;
            assert_eq!(sources["ok"], true, "{sources}");
            for (id, text, selected) in [
                ("first", "edited first", &first),
                ("second", "second", &second),
            ] {
                let source = sources["result"]["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|source| source["messageId"] == id)
                    .unwrap();
                assert_eq!(source["content"]["text"], text);
                assert_eq!(source["content"]["quotes"], json!([selected["quote"]]));
                assert_eq!(source["inputSelectionSources"], json!([selected["source"]]));
                assert_eq!(
                    source["inputSelections"][PREPARE],
                    json!([selected["selector"]])
                );
            }
            expected_sources = Some(sources["result"].clone());
            accepted = Some(original);
        } else {
            let sources = peer
                .rpc(
                    "session.sources.query",
                    json!({"sessionId":SESSION,"turnId":"turn"}),
                )
                .await;
            assert_eq!(sources["ok"], true, "{sources}");
            assert_eq!(Some(sources["result"].clone()), expected_sources);
            let replay = peer
                .rpc("turn.message.submit", accepted.clone().unwrap())
                .await;
            assert_eq!(replay["result"]["disposition"], "steering", "{replay}");
            let mut stale = accepted.clone().unwrap();
            stale["originHostEpoch"] = hello["hostEpoch"].clone();
            stale["messageId"] = json!("stale-reference");
            let rejected = peer.rpc("turn.message.submit", stale).await;
            assert_eq!(
                rejected["ok"], false,
                "old source must not bind same-name replacement: {rejected}"
            );
        }
        assert_eq!(original_calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            replacement_calls.load(Ordering::SeqCst),
            0,
            "accepted delivery/replay never prepares against a replacement"
        );
        peer.close().await;
        drop(replacement);
        plugin.binding.lock().unwrap().take();
        // Reopening requires the listener's execution/plugin/storage drain;
        // closing this Peer alone only settles its connection.
        drop(cleanup);
        server.await.unwrap().unwrap();
        drop(host);
    }
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    let log = fixture.log().await;
    let first = log.root_message(SESSION, "first").await.unwrap().unwrap();
    assert!(matches!(
        first.opening().event.fact,
        Fact::MessageSteered {
            source: Some(_),
            ..
        }
    ));
    assert_eq!(
        first.source().submitted_placement,
        maka_runtime::message::Placement::NextTurn
    );
    assert_eq!(first.source().unprepared_content.text, "edited first");
    log.shutdown().await.unwrap();
}
