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
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn session(peer: &mut Peer, id: &str) -> Value {
    let result = peer
        .rpc(
            "session.catalog.query",
            json!({"kind":"get", "sessionId":id}),
        )
        .await;
    assert_eq!(result["ok"], true, "{result}");
    result["result"]["session"].clone()
}

fn reply(text: &str) -> Value {
    json!({"index":0,"delta":{"content":text},"finish_reason":"stop"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn automatic_title_survives_turn_stop_and_cannot_overwrite_a_manual_name() {
    tokio::time::timeout(Duration::from_secs(60), scenario())
        .await
        .unwrap();
}

async fn scenario() {
    let fixture = HostFixture::new("maka-session-title-");
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&fixture.workspace)
            .status()
            .unwrap()
            .success()
    );
    let (provider, mut requests) = Provider::controlled().await;
    let model = configure(&fixture, &provider.base_url).await;
    // The old create path persisted custom names with title_is_manual=false.
    let legacy_input = json!({
        "sessionId":"legacy", "name":"Existing custom title",
        "workspace":{"kind":"host_path","path":fixture.workspace},
        "modelTarget":{"kind":"default"}, "sandboxMode":"read-only", "approvalPolicy":{"kind":"never"}
    });
    let prepared = maka_runtime_host::session::PreparedSession::new(
        serde_json::from_value(legacy_input.clone()).unwrap(),
    )
    .unwrap();
    let mut legacy = prepared.bind(
        maka_protocol::session::WorkspaceProjection {
            target: maka_protocol::session::WorkspaceTarget::HostPath {
                path: fixture.workspace.to_string_lossy().into_owned(),
            },
            host_cwd: fixture.workspace.to_string_lossy().into_owned(),
        },
        model.clone(),
        maka_protocol::session::SandboxMode::ReadOnly,
    );
    // Exact v4 creation identity from the pre-title branch, kept as a fixture.
    let v4 = json!([
        "session.create.v4", "legacy", ["host_path", fixture.workspace],
        "Existing custom title", [], ["default"], ["model_default"], null,
        "read-only", {"kind":"never"}, "agent", "default"
    ]);
    let fingerprint = format!("sha256:{:x}", Sha256::digest(v4.to_string().as_bytes()));
    legacy.title_is_manual = false;
    let log = fixture.log().await;
    log.create_session("legacy", &fingerprint, &legacy, 1)
        .await
        .unwrap();
    log.close().await.unwrap();
    let database = fixture
        .owner()
        .canonical_path()
        .join(maka_event_log::root::ROOT_DATABASE);
    let host = Host::open(fixture.owner()).await.unwrap();
    #[cfg(unix)]
    let endpoint = fixture.workspace.parent().unwrap().join("title.sock");
    #[cfg(windows)]
    let endpoint =
        std::path::PathBuf::from(format!(r"\\.\pipe\maka-title-{}", uuid::Uuid::new_v4()));
    let stop = CancellationToken::new();
    let server = tokio::spawn(
        LocalListener::bind(&endpoint)
            .unwrap()
            .serve(host.clone(), stop.clone()),
    );
    let (mut peer, hello) = Peer::handshake(host, "title").await;
    peer.wait_for_plugins().await;
    let retried = peer.rpc("session.create", legacy_input).await;
    assert_eq!(retried["ok"], true, "{retried}");
    assert_eq!(retried["result"]["name"], "Existing custom title");
    let legacy_started = peer.rpc("turn.message.submit", json!({
        "originHostEpoch":hello["hostEpoch"], "sessionId":"legacy", "messageId":"legacy-first",
        "placement":"current_turn", "content":{"text":"Keep my existing title"}
    })).await;
    assert_eq!(legacy_started["ok"], true, "{legacy_started}");
    let legacy_request = requests.recv().await.unwrap();
    assert!(
        !legacy_request
            .body
            .to_string()
            .contains("Generate a short session title")
    );
    legacy_request.reply.send(reply("legacy response")).unwrap();
    let updated = peer
        .rpc(
            "connection.catalog.update",
            json!({
                "expected":{"connectionId":model.connection_id,"revision":2},
                "changes":{"name":"Recovery fixture","configuration":{"baseUrl":provider.base_url},"enabled":true,
                    "enabledModelIds":["fixture-model"],"modelOverrides":{"fixture-model":{"contextWindow":200000,
                        "thinkingLevels":["high","minimal"],"defaultThinkingLevel":"high"
                    }}}
            }),
        )
        .await;
    assert_eq!(updated["result"]["kind"], "committed", "{updated}");
    let named = peer.rpc("session.create", json!({
        "sessionId":"named", "name":"My initial title",
        "workspace":{"kind":"host_path","path":fixture.workspace},
        "modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}
    })).await;
    assert_eq!(named["ok"], true, "{named}");
    for (id, manual) in [("automatic", false), ("manual", true)] {
        let created = peer.rpc("session.create", json!({
            "sessionId":id, "workspace":{"kind":"host_path","path":fixture.workspace},
            "sandboxMode":"read-only", "approvalPolicy":{"kind":"never"},
            "modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}
        })).await;
        assert_eq!(created["ok"], true, "{created}");
        assert!(
            requests.try_recv().is_err(),
            "empty sessions do not request titles"
        );
        let input = json!({"originHostEpoch":hello["hostEpoch"], "sessionId":id, "messageId":"first",
            "placement":"current_turn", "content":{"text":"Help me investigate the provider search interface"}});
        let started = peer.rpc("turn.message.submit", input.clone()).await;
        assert_eq!(started["ok"], true, "{started}");
        let first = requests.recv().await.unwrap();
        let second = requests.recv().await.unwrap();
        let (title, main) = if first
            .body
            .to_string()
            .contains("Generate a short session title")
        {
            (first, second)
        } else {
            (second, first)
        };
        assert!(
            title
                .body
                .to_string()
                .contains("Generate a short session title")
        );
        assert!(
            title
                .body
                .get("tools")
                .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
        );
        assert_eq!(title.body["reasoning_effort"], "minimal", "{}", title.body);
        assert_eq!(main.body["reasoning_effort"], "high", "{}", main.body);
        let turn = peer
            .rpc(
                "turn.query",
                json!({"sessionId":id,"turnId":started["result"]["turnId"]}),
            )
            .await;
        let stopped = peer.rpc("turn.stop", json!({"sessionId":id,"turnId":started["result"]["turnId"],"runId":turn["result"]["runId"]})).await;
        assert_eq!(stopped["ok"], true, "{stopped}");
        let _ = main.reply.send(reply("main response"));
        if manual {
            loop {
                let current = session(&mut peer, id).await;
                let renamed = peer.rpc("session.metadata.update", json!({"sessionId":id,"expectedRevision":current["revision"],"patch":{"name":"My chosen title"}})).await;
                assert_eq!(renamed["ok"], true, "{renamed}");
                match renamed["result"]["kind"].as_str() {
                    Some("committed") => break,
                    Some("revision_conflict") => continue,
                    _ => panic!("{renamed}"),
                }
            }
        }
        title
            .reply
            .send(reply("Provider search investigation"))
            .unwrap();
        // Read-only canonical evidence proves the title finished before checking
        // the manual-name race; shutdown cancellation cannot make the test pass.
        loop {
            let connection = rusqlite::Connection::open_with_flags(
                &database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let settled: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM event_log a JOIN event_log s
                 ON json_extract(s.event_json,'$.request_id')=a.event_id
                 WHERE a.kind='auxiliary_model_started' AND s.kind='auxiliary_model_settled'
                 AND json_extract(a.event_json,'$.source.kind')='session_title'
                 AND json_extract(a.event_json,'$.session_id')=?1)",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            if settled {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if !manual {
            loop {
                if session(&mut peer, id).await["name"] == "Provider search investigation" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let retry = peer.rpc("turn.message.submit", input).await;
        assert_eq!(retry["result"]["turnId"], started["result"]["turnId"]);
    }
    peer.close().await;
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let log = fixture.log().await;
    let legacy = log
        .get_session::<maka_runtime_host::session::SessionConfiguration>("legacy")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy.configuration.name, "Existing custom title");
    let named = log
        .get_session::<maka_runtime_host::session::SessionConfiguration>("named")
        .await
        .unwrap()
        .unwrap();
    assert!(named.configuration.title_is_manual);
    assert_eq!(named.configuration.name, "My initial title");
    let auto = log
        .get_session::<maka_runtime_host::session::SessionConfiguration>("automatic")
        .await
        .unwrap()
        .unwrap();
    let manual = log
        .get_session::<maka_runtime_host::session::SessionConfiguration>("manual")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(auto.configuration.name, "Provider search investigation");
    assert!(!auto.configuration.title_is_manual);
    assert_eq!(manual.configuration.name, "My chosen title");
    assert!(manual.configuration.title_is_manual);
    let report = log
        .model_attempts(
            maka_event_log::usage::Query {
                from: 0.0,
                to: f64::MAX,
                session_id: None,
                through: None,
            },
            0,
            100,
        )
        .await
        .unwrap();
    assert_eq!(
        report
            .attempts
            .iter()
            .filter(|row| matches!(
                row.origin,
                maka_runtime::accounting::Origin::Auxiliary {
                    source: maka_runtime::accounting::AuxiliarySource::SessionTitle { .. }
                }
            ))
            .count(),
        2
    );
    assert!(
        requests.try_recv().is_err(),
        "exact retry does not issue another title request"
    );
}
