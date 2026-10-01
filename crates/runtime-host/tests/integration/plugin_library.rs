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

use super::support::{host_fixture::HostFixture, peer::Peer};
use maka_protocol::plugin::BuiltinProjection;
use maka_runtime_host::server::Host;
use serde_json::{Value, json};

async fn query(peer: &mut Peer, view: &str) -> Value {
    let reply = peer
        .rpc("plugin.platform.query", json!({"view":view,"limit":64}))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply["result"]["nextCursor"].is_null());
    reply["result"]["items"].clone()
}

async fn builtin(peer: &mut Peer, id: &str) -> BuiltinProjection {
    let items = query(peer, "builtins").await;
    serde_json::from_value(
        items
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["extensionId"] == id)
            .unwrap()
            .clone(),
    )
    .unwrap()
}

async fn manage(peer: &mut Peer, id: &str, input: Value) -> Value {
    let binding = json!({"packageId":id,"method":"manage"});
    let bound = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(bound["ok"], true, "{bound}");
    let opened = peer
        .rpc("plugin.remote", json!({"kind":"open_document"}))
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let document = &opened["result"]["document"];
    let reply = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"call","binding":binding,
        "target":bound["result"]["target"],"document":document,"input":input}),
        )
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    let closed = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"close_document","document":document}),
        )
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_plugin_stays_discoverable_after_removal_and_reopens_before_readding_defaults() {
    let fixture = HostFixture::new("maka-plugin-library-");
    let id = maka_external_agent::plugin::ID;
    let agents = json!([{"id":"fixture.executor","displayName":"Fixture executor","executable":fixture.workspace.join("fixture-executor"),"args":[],"env":{}}]);
    for reopened in [false, true] {
        let host = Host::open(fixture.owner()).await.unwrap();
        let mut peer = Peer::new(host.clone(), "plugin-library").await;
        peer.wait_for_plugins().await;
        let plugin = builtin(&mut peer, id).await;
        assert_eq!(plugin.description.name.resolve("zh-CN"), "执行器");
        assert!(!plugin.defaults.is_empty());
        assert!(
            query(&mut peer, "packages")
                .await
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["extensionId"] != id)
        );
        if !reopened {
            let current = manage(&mut peer, id, json!({"kind":"read"})).await;
            let configured = manage(
                &mut peer,
                id,
                json!({"kind":"configure",
                "expectedRevision":current["revision"],"agents":agents}),
            )
            .await;
            assert_eq!(configured["agents"], agents);
            let removed = peer
                .rpc(
                    "plugin.composition.apply",
                    json!({
                        "baseGeneration":plugin.base_generation,
                        "operations":[{"type":"remove","entryId":id}]
                    }),
                )
                .await;
            assert_eq!(removed["ok"], true, "{removed}");
            peer.wait_for_plugins().await;
            let available = builtin(&mut peer, id).await;
            assert!(available.base_generation > plugin.base_generation);
            assert_eq!(
                serde_json::to_value(available.defaults).unwrap(),
                serde_json::to_value(plugin.defaults).unwrap()
            );
            assert!(
                query(&mut peer, "terminal_views")
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["packageId"] != id)
            );
        } else {
            assert!(
                query(&mut peer, "entries")
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["packageId"] != id)
            );
            let stale = peer.rpc("plugin.composition.apply", json!({
                "baseGeneration":plugin.base_generation.saturating_sub(1), "operations":plugin.defaults
            })).await;
            assert_eq!(stale["ok"], false, "{stale}");
            assert_eq!(stale["error"]["code"], "operation_conflict");
            let added = peer
                .rpc(
                    "plugin.composition.apply",
                    json!({
                        "baseGeneration":plugin.base_generation,"operations":plugin.defaults
                    }),
                )
                .await;
            assert_eq!(added["ok"], true, "{added}");
            peer.wait_for_plugins().await;
            assert!(
                query(&mut peer, "entries")
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["packageId"] == id && item["status"] == "active")
            );
            let configured = manage(&mut peer, id, json!({"kind":"read"})).await;
            assert_eq!(configured["agents"], agents);
            assert!(
                query(&mut peer, "executors")
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["id"] == "fixture.executor")
            );
            assert!(
                query(&mut peer, "terminal_views")
                    .await
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["packageId"] == id && item["descriptor"]["launch"] == true)
            );
        }
        peer.close().await;
        drop(host);
    }
}
