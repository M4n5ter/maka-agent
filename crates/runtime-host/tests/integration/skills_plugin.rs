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

use super::support::{
    host_fixture::HostFixture,
    message_recovery::{Provider, configure},
    peer::Peer,
};
use maka_runtime_host::server::{Host, local::LocalListener};
use serde_json::json;
use std::time::Duration;
mod authoring;
pub(super) mod client;
mod native;

pub(super) async fn converged(peer: &mut Peer) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = peer
                .rpc("plugin.platform.query", json!({"view":"status"}))
                .await;
            assert_eq!(state["ok"], true, "{state}");
            if state["result"]["convergence"] == "converged" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

pub(super) async fn disabled(peer: &mut Peer, disabled: bool) {
    let result = peer
        .rpc(
            "plugin.composition.apply",
            json!({
                "operations":[
                    {"type":"update","entryId":"maka.skills","patch":{"disabled":disabled}}
                ]
            }),
        )
        .await;
    assert_eq!(result["ok"], true, "{result}");
    converged(peer).await;
}
