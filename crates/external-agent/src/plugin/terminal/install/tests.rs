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
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

fn agent() -> Agent {
    Agent {
        id: "antigravity-acp".into(),
        display_name: "Antigravity".into(),
        executable: std::env::temp_dir()
            .join("fixture")
            .to_string_lossy()
            .into_owned(),
        args: vec![],
        env: BTreeMap::new(),
    }
}
struct Controlled {
    event: Mutex<Option<Value>>,
    closing: Option<oneshot::Sender<()>>,
    settled: oneshot::Receiver<()>,
    cancelled: Arc<AtomicBool>,
    cleanup_error: bool,
}
impl remote::Stream for Controlled {
    fn next(&self) -> BoxFuture<'_, Result<Option<Value>, Error>> {
        Box::pin(async move { Ok(self.event.lock().unwrap().take()) })
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    fn close(mut self: Box<Self>) -> BoxFuture<'static, Result<(), Error>> {
        Box::pin(async move {
            self.closing.take().unwrap().send(()).unwrap();
            self.settled.await.unwrap();
            if self.cleanup_error {
                Err(Error::CleanupUnconfirmed)
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn installation_result_and_cancellation_wait_for_actual_stream_settlement() {
    for (cancel, cleanup_error) in [(false, false), (true, false), (false, true)] {
        let attempt = Attempt::new();
        if cancel {
            attempt.stop.cancel();
        }
        let (state, _) = watch::channel(Some(attempt.clone()));
        let (closing, closed) = oneshot::channel();
        let (settle, settled) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream = Controlled {
            event: Mutex::new(Some(json!({"kind":"installed","agent":agent()}))),
            closing: Some(closing),
            settled,
            cancelled: cancelled.clone(),
            cleanup_error,
        };
        let worker_state = state.clone();
        let worker = tokio::spawn(async move {
            consume(
                Box::new(stream),
                &worker_state,
                &attempt,
                &CancellationToken::new(),
                &CancellationToken::new(),
            )
            .await
        });
        closed.await.unwrap();
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(state.borrow().as_ref().unwrap().active());
        assert!(state.borrow().as_ref().unwrap().agent.is_none());
        settle.send(()).unwrap();
        let result = worker.await.unwrap();
        let current = state.borrow().clone().unwrap();
        if cleanup_error {
            assert!(matches!(result, Err(Error::CleanupUnconfirmed)));
            assert!(current.phase == Phase::Unknown && current.agent.is_none());
        } else if cancel {
            assert!(matches!(result, Err(Error::Cancelled)));
            assert!(current.phase == Phase::Cancelled && current.agent.is_none());
        } else {
            result.unwrap();
            assert!(current.phase == Phase::Installed);
            assert_eq!(current.agent, Some(agent()));
        }
    }
}

#[test]
fn only_settled_installations_offer_explicit_frozen_adoption() {
    let configuration = Configuration {
        revision: Some(4),
        agents: vec![agent()],
        activation_error: None,
    };
    for phase in [
        Phase::Installing,
        Phase::Cancelling,
        Phase::Installed,
        Phase::Cancelled,
        Phase::Failed,
        Phase::Unknown,
    ] {
        let mut attempt = Attempt::new();
        attempt.phase = phase;
        attempt.agent = Some(agent());
        for locale in ["en", "zh-CN", "zh-TW"] {
            let shown = view(
                &Words::new(locale),
                &configuration,
                Some(attempt.clone()),
                true,
            );
            shown.validate().unwrap();
            let action = shown.action(&format!("adopt-install-{}", attempt.id));
            assert_eq!(action.is_some(), phase == Phase::Installed);
            if let Some(action) = action {
                assert!(action.confirm.as_ref().unwrap().destructive);
                assert!(action.recovery.is_none());
            }
            if phase == Phase::Unknown {
                assert!(shown.action("install-antigravity").is_none());
            }
        }
    }
}
