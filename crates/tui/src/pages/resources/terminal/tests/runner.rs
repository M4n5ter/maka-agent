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

use super::super::*;
use crate::apps::io::transcript::test_peer::Peer;
use maka_client::Client;
use maka_protocol::subscription::{SubscriptionOpenInput, TranscriptPolicy};
use serde_json::{Value, json};

async fn connected() -> (Client, Peer) {
    let (client, mut peer) = Peer::connect().await;
    let c = client.clone();
    let open = tokio::spawn(async move {
        c.open_subscription(SubscriptionOpenInput {
            session_id: "session".into(),
            transcript: TranscriptPolicy::None,
        })
        .await
    });
    let request = peer.read().await;
    peer.reply_value(&request,json!({"hostEpoch":"epoch","subscriptionId":"sub","nextSequence":1,"activeAssistantStreams":[],"transcript":null,"snapshot":{"schemaVersion":5,"session":{"sessionId":"session","metadataRevision":1,"status":"active","createdAt":0,"isArchived":false},"projectionRevision":1,"rootTurn":null,"goal":null,"queue":{"hostEpoch":"epoch","queueRevision":0,"steering":[],"followup":[]},"interactions":{"pending":[]}}})).await;
    open.await.unwrap().unwrap();
    let c = client.clone();
    let ready = tokio::spawn(async move { c.ready_subscription("sub").await });
    let request = peer.read().await;
    peer.reply_value(&request, json!({"subscriptionId":"sub"}))
        .await;
    ready.await.unwrap().unwrap();
    (client, peer)
}
fn cut(request: &Value) -> Value {
    json!({"controllerId":request["input"]["controllerId"],"nextSequence":1,"pty":{"sessionId":"session","ref":"maka://runtime/background-tasks/r","sequence":0,"buffer":"ready","size":{"cols":80,"rows":24}}})
}
#[tokio::test]
async fn late_acquire_releases_exact_controller_without_accepting_input() {
    let (client, mut peer) = connected().await;
    let (mut runner, mut events) = Runner::new();
    let mut target = super::target();
    target.root = client.identity.root_id.clone();
    runner.reconcile(&client, Some(target));
    let acquire = peer.read().await;
    assert_eq!(acquire["operation"], "runtime.resource.controller.acquire");
    runner.reconcile(&client, None);
    peer.reply_value(&acquire, cut(&acquire)).await;
    let release = peer.read().await;
    assert_eq!(release["operation"], "runtime.resource.controller.release");
    assert_eq!(
        release["input"]["controllerId"],
        acquire["input"]["controllerId"]
    );
    peer.reply_value(
        &release,
        json!({"controllerId":release["input"]["controllerId"],"released":true}),
    )
    .await;
    assert!(matches!(
        events.recv().await.unwrap().update,
        Update::Closed(true)
    ));
    assert!(runner.shutdown().await);
    client.disconnect();
}
#[tokio::test]
async fn accepted_input_settles_before_release_and_unconfirmed_cleanup_stays_failed() {
    let (client, mut peer) = connected().await;
    let (mut runner, mut events) = Runner::new();
    let mut target = super::target();
    target.root = client.identity.root_id.clone();
    runner.reconcile(&client, Some(target));
    let acquire = peer.read().await;
    peer.reply_value(&acquire, cut(&acquire)).await;
    let event = events.recv().await.unwrap();
    assert!(matches!(event.update, Update::Acquired(_)));
    runner
        .control(
            event.token,
            Control::Write(PtyControl::Input {
                input: "exact input\r".into(),
            }),
        )
        .unwrap();
    let writing = peer.read().await;
    assert_eq!(writing["input"]["sequence"], 1);
    assert_eq!(writing["input"]["control"]["input"], "exact input\r");
    runner.reconcile(&client, None);
    let mut pending = Box::pin(peer.read());
    assert!(futures_util::poll!(&mut pending).is_pending());
    drop(pending);
    peer.reply_value(
        &writing,
        json!({"controllerId":writing["input"]["controllerId"],"sequence":1}),
    )
    .await;
    let release = peer.read().await;
    assert_eq!(release["operation"], "runtime.resource.controller.release");
    peer.reject(
        &release,
        maka_protocol::OperationError {
            code: maka_protocol::OperationErrorCode::OperationUnavailable,
            message: "cleanup unavailable".into(),
        },
    )
    .await;
    assert!(!runner.shutdown().await);
    client.disconnect();
}
