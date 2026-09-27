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

use super::support::{client_probe::ClientFixture, peer::Peer};
use maka_protocol::session::{
    SandboxMode, WorkspaceProjection, WorkspaceTarget, workspace_context as api,
};
use maka_runtime_host::{
    server::Host,
    session::{PreparedSession, SessionModel},
};
use serde_json::{Value, json};

fn success(reply: Value) -> Value {
    assert_eq!(reply["ok"], true, "{reply}");
    reply["result"].clone()
}

#[tokio::test]
async fn native_workspace_capture_contains_actual_context_and_rejects_retired_boundaries() {
    let fixture = ClientFixture::new("maka-context-capture-");
    let cwd =
        maka_fs_tools::workspace::project::host_path(&fixture.workspace.canonicalize().unwrap())
            .unwrap()
            .to_owned();
    let configuration = PreparedSession::new(serde_json::from_value(json!({
        "sessionId":"session","workspace":{"kind":"host_path","path":cwd},"modelTarget":{"kind":"default"}
    })).unwrap()).unwrap().bind(
        WorkspaceProjection { target: WorkspaceTarget::HostPath { path: cwd.clone() }, host_cwd: cwd },
        SessionModel { connection_id: "fixture".into(), connection_slug: "fixture".into(), model: "fixture".into() }, SandboxMode::ReadOnly);
    let log = fixture.log().await;
    log.create_session("session", "session", &configuration, 1)
        .await
        .unwrap();
    log.close().await.unwrap();
    std::fs::write(
        fixture.workspace.join("真实.txt"),
        "actual quoted context 🦀",
    )
    .unwrap();
    let host = Host::open(fixture.owner()).await.unwrap();
    let mut peer = Peer::new(host.clone(), "context-client").await;
    let page = success(
        peer.rpc(
            "session.workspace.query",
            json!({"sessionId":"session","directory":"","filter":"真实","cursor":null}),
        )
        .await,
    );
    let page = api::decode_page(&page).unwrap();
    assert_eq!(page.entries.len(), 1);
    let capture = api::Capture {
        basis: page.basis,
        path: page.entries[0].path.clone(),
        kind: api::Kind::File,
    };
    let captured = success(
        peer.rpc(
            "session.workspace.capture",
            serde_json::to_value(&capture).unwrap(),
        )
        .await,
    );
    let captured = api::decode_captured(&captured).unwrap();
    assert!(captured.quote.text.contains("actual quoted context 🦀"));
    assert!(captured.directory_reference.is_none());
    std::fs::write(fixture.workspace.join("真实.txt"), "new content").unwrap();
    assert!(captured.quote.text.contains("actual quoted context 🦀"));
    let destination = fixture.workspace.join("different");
    std::fs::create_dir(&destination).unwrap();
    success(
        peer.rpc(
            "session.workspace.relocate",
            json!({"sessionId":"session","expectedRevision":1,
        "workspace":{"kind":"host_path","path":destination}}),
        )
        .await,
    );
    let stale = peer
        .rpc(
            "session.workspace.capture",
            serde_json::to_value(&capture).unwrap(),
        )
        .await;
    assert_eq!(stale["error"]["code"], "candidate_set_stale", "{stale}");
    let new = success(
        peer.rpc(
            "session.workspace.query",
            json!({"sessionId":"session","directory":"","filter":"","cursor":null}),
        )
        .await,
    );
    assert_ne!(new["basis"], serde_json::to_value(capture.basis).unwrap());
    peer.close().await;
    drop(host);
}
