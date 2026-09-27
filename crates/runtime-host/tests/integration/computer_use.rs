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
    client_probe::ClientFixture,
    message_recovery::{Provider, configure},
    peer::Peer,
};
use maka_runtime_host::server::{Host, local::LocalListener};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
mod persistence;

fn call(id: &str, name: &str, input: Value) -> Value {
    json!({"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":input.to_string()}}]},"finish_reason":"tool_calls"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn computer_tools_use_canonical_approval_and_refuse_before_native_dispatch() {
    scenario(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires an interactive desktop; observes application inventory without input"]
async fn native_computer_observation_crosses_host_approval_and_journal() {
    scenario(true).await;
}

async fn scenario(native: bool) {
    tokio::time::timeout(Duration::from_secs(45), async {
        let fixture = ClientFixture::new("maka-computer-");
        let (provider, mut requests) = Provider::controlled().await;
        let model = configure(&fixture, &provider.base_url).await;
        let owner = fixture.owner();
        let database = owner.canonical_path().join(maka_event_log::root::ROOT_DATABASE);
        let host = Host::open(owner).await.unwrap();
        let mut database = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(database).read_only(true)).await.unwrap();
        let stop = CancellationToken::new();
        let cleanup = stop.clone().drop_guard();
        #[cfg(unix)]
        let endpoint = fixture.workspace.parent().unwrap().join("computer.sock");
        #[cfg(windows)]
        let endpoint = std::path::PathBuf::from(format!(r"\\.\pipe\maka-computer-{}", uuid::Uuid::new_v4()));
        let server = tokio::spawn(LocalListener::bind(&endpoint).unwrap().serve(host.clone(), stop));
        let mut peer = Peer::new(host, "computer-client").await;
        peer.wait_for_plugins().await;
        let mut cases = vec![
            ("explore", "read-only", "on-request", "Explore mode does not allow Computer Use"),
            ("no-prompt", "workspace-write", "never", "forbids prompting"),
            ("deny", "workspace-write", "on-request", "approval was denied"),
        ];
        if native { cases.push(("allow", "workspace-write", "on-request", "")); }
        for (session, mode, policy, expected) in cases {
            let created = peer.rpc("session.create", json!({
                "sessionId":session,"sandboxMode":mode,"approvalPolicy":{"kind":policy},
                "workspace":{"kind":"host_path","path":fixture.workspace},
                "modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}
            })).await;
            assert_eq!(created["ok"], true, "{created}");
            assert_eq!(peer.rpc("turn.start", json!({"sessionId":session,"turnId":"attempt","content":{"text":"Inspect available applications"}})).await["ok"], true);
            requests.recv().await.unwrap().reply.send(call("discover", "tool_search", json!({"query":"cua_repl"}))).unwrap();
            let request = requests.recv().await.unwrap();
            assert!(request.body["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "cua_repl"));
            request.reply.send(call("apps", "cua_repl", json!({"code":"await cua.listApps()"}))).unwrap();
            if session == "deny" || session == "allow" {
                let (id, payload): (String, String) = loop {
                    let pending = sqlx::query_as("SELECT request_id, json_extract(record_json, '$.request') FROM interaction_requests WHERE json_extract(record_json, '$.sessionId') = ?")
                        .bind(session).fetch_optional(&mut database).await.unwrap();
                    if let Some(found) = pending { break found; }
                    tokio::task::yield_now().await;
                };
                let payload: Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(payload["kind"], "client_capability");
                assert_eq!(payload["target"]["capability"], "computer_use");
                assert_eq!(payload["target"]["providerId"], "maka_computer_use_host");
                let native: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_events WHERE json_extract(event_json,'$.fact.call.origin.kind')='host_sdk' AND json_extract(event_json,'$.fact.name')='cua_operation' AND json_extract(event_json,'$.fact.input.method')='listApps'")
                    .fetch_one(&mut database).await.unwrap();
                assert_eq!(native, 0, "no native dispatch before approval");
                let answer = peer.rpc("interaction.answer", json!({"sessionId":session,"interactionId":id,"answer":{"kind":"client_capability","decision":if session == "allow" { "allow" } else { "deny" }}})).await;
                assert_eq!(answer["ok"], true, "{answer}");
            }
            let request = requests.recv().await.unwrap();
            let output = request.body["messages"].as_array().unwrap().iter().find(|m| m["tool_call_id"]=="apps").unwrap()["content"].as_str().unwrap();
            if session == "allow" {
                assert!(!output.contains("tool failed:"), "{output}");
                let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_events WHERE json_extract(event_json,'$.fact.call.origin.kind')='host_sdk' AND json_extract(event_json,'$.fact.name')='cua_operation' AND json_extract(event_json,'$.fact.input.method')='listApps'")
                    .fetch_one(&mut database).await.unwrap();
                assert_eq!(count, 1, "approved call has its own canonical native dispatch");
                let outcome: String = sqlx::query_scalar("SELECT json_extract(event_json,'$.fact.outcome.kind') FROM runtime_events WHERE kind='tool_settled' AND operation_id IN (SELECT operation_id FROM runtime_events WHERE json_extract(event_json,'$.fact.call.origin.kind')='host_sdk' AND json_extract(event_json,'$.fact.name')='cua_operation' AND json_extract(event_json,'$.fact.input.method')='listApps')")
                    .fetch_one(&mut database).await.unwrap();
                assert_eq!(outcome, "succeeded", "native observation must settle successfully");
            } else {
                assert!(output.contains(expected), "{session}: {output}");
            }
            request.reply.send(json!({"index":0,"delta":{"content":"finished"},"finish_reason":"stop"})).unwrap();
            loop {
                let turn = peer.rpc("turn.query", json!({"sessionId":session,"turnId":"attempt"})).await;
                match turn["result"]["status"].as_str() {
                    Some("completed") => break,
                    Some("failed" | "cancelled") => panic!("{turn}"),
                    _ => tokio::task::yield_now().await,
                }
            }
        }
        let native_dispatches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_events WHERE json_extract(event_json,'$.fact.call.origin.kind')='host_sdk' AND json_extract(event_json,'$.fact.name')='cua_operation' AND json_extract(event_json,'$.fact.input.method')='listApps'")
            .fetch_one(&mut database).await.unwrap();
        assert_eq!(native_dispatches, i64::from(native));
        peer.close().await;
        drop(cleanup);
        server.await.unwrap().unwrap();
    }).await.expect("Computer Use approval settles");
}
