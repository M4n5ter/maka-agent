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
use maka_protocol::{
    Operation,
    configuration::ConnectionCatalogQueryInput,
    session::{Backend, SessionCatalogQueryInput, SessionCatalogQueryResult, WorkspaceTarget},
};
use serde_json::json;

const EXECUTOR: &str = "example.executor197";
const NAME: &str = "Executor only 197";

async fn install(client: &maka_client::Client, directory: &std::path::Path) {
    let package = directory.join("executor-plugin");
    std::fs::create_dir(&package).unwrap();
    // Reuse the public workflow executor; its optional background workflow stays
    // idle because this fresh Root has no queued intent or authorization grant.
    std::fs::write(
        package.join("host.mjs"),
        include_str!("../../fixtures/executor-plugin.mjs"),
    )
    .unwrap();
    std::fs::write(
        package.join("maka.extension.json"),
        json!({"schemaVersion":1,"id":EXECUTOR,
        "runtime":{"entry":"host.mjs","sdkVersion":3}})
        .to_string(),
    )
    .unwrap();
    client
        .request(
            Operation::PluginPackageInstall,
            json!({"sourcePath":package}),
        )
        .await
        .unwrap();
    client
        .request(
            Operation::PluginCompositionApply,
            json!({"operations":[{"type":"insert","rootId":"profile",
        "entry":{"id":"executor197","packageId":EXECUTOR}}]}),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = client
                .request(
                    Operation::ExecutorCatalogQuery,
                    json!({"scope":"profile","query":"Public workflow","cursor":null}),
                )
                .await
                .unwrap();
            if page["kind"] == "page"
                && page["page"]["executors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["id"] == EXECUTOR && item["displayName"] == "Public workflow")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("public executor did not become available");
}

fn name(tui: &mut Pty) {
    let label = "Session name (optional)";
    tui.wait_for(label);
    let snapshot = tui.screen.snapshot().unwrap().screen;
    let (row, column) = snapshot
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find(label).map(|byte| (row + 1, line[..byte].width())))
        .expect("name label above its actual input slot");
    tui.click_at(row, column);
    tui.send(format!("\x1b[200~{NAME}\x1b[201~").as_bytes());
    tui.wait_for(NAME);
}

#[test]
fn home_creates_and_runs_an_executor_session_without_any_model_connection() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("executor workspace 中文");
    std::fs::create_dir(&workspace).unwrap();
    let expected = workspace
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let mut host = super::super::candidate::CandidateFixture::new(directory.path().join("root"));
    host.child = Some(
        Command::new(env!("CARGO_BIN_EXE_maka"))
            .args(["host", "serve", "--root"])
            .arg(&host.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    host.wait_for_registration();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Transport only: no model_client, login, connection or default-model fixture.
    let client = runtime.block_on(support::client(&host.root));
    let connections = runtime
        .block_on(client.connection_catalog(ConnectionCatalogQueryInput::Start))
        .unwrap();
    assert!(connections["items"].as_array().unwrap().is_empty());
    assert!(connections["defaultTarget"].is_null());
    assert!(connections["nextCursor"].is_null());
    runtime.block_on(install(&client, directory.path()));
    let mut tui = Pty::spawn_at(&["--root", host.root.to_str().unwrap()], Some(&workspace));
    tui.wait_for("New executor session");
    tui.click_page_text("New executor session");
    tui.wait_for("Session name (optional)");
    tui.wait_for("Public workflow");
    name(&mut tui);
    tui.click_page_text("[ ] Public workflow");
    tui.wait_for("[x] Public workflow");
    let SessionCatalogQueryResult::Page {
        sessions,
        next_cursor,
        ..
    } = runtime
        .block_on(client.session_catalog(SessionCatalogQueryInput::ListStart))
        .unwrap()
    else {
        panic!("session catalog")
    };
    assert!(
        sessions.is_empty() && next_cursor.is_none(),
        "browsing and selection cannot create a Session"
    );
    assert_eq!(
        runtime
            .block_on(client.connection_catalog(ConnectionCatalogQueryInput::Start))
            .unwrap(),
        connections
    );
    tui.click_last_text("Create");
    tui.wait_until(|screen| {
        screen.contains(NAME) && screen.contains("No messages yet.") && screen.contains("Message…")
    });
    assert!(
        !tui.screen
            .snapshot()
            .unwrap()
            .screen
            .contains("Search commands…")
    );
    let SessionCatalogQueryResult::Page {
        sessions,
        next_cursor,
        ..
    } = runtime
        .block_on(client.session_catalog(SessionCatalogQueryInput::ListStart))
        .unwrap()
    else {
        panic!("created catalog")
    };
    assert!(next_cursor.is_none());
    assert_eq!(sessions.len(), 1, "exactly the page-created Session exists");
    let created = &sessions[0];
    assert_eq!(created.name, NAME);
    assert_eq!(created.backend, Backend::PluginExecutor);
    assert_eq!(
        created.executor_id.as_ref().map(|id| id.as_str()),
        Some(EXECUTOR)
    );
    assert!(created.llm_connection_id.is_none());
    assert_eq!(created.workspace.host_cwd, expected);
    assert_eq!(
        created.workspace.target,
        WorkspaceTarget::HostPath {
            path: expected.clone()
        }
    );
    let id = created.id.clone();
    // Creation must lead to a usable ordinary composer, even with no model default.
    tui.click_page_text("Message…");
    tui.send(b"\x1b[200~Run the registered executor.\x1b[201~");
    tui.wait_for("Run the registered executor.");
    tui.send(b"\r");
    tui.wait_for("Public plugin execution completed.");
    let settled = runtime.block_on(client.session(&id)).unwrap().unwrap();
    assert_eq!(
        settled.executor_id.as_ref().map(|id| id.as_str()),
        Some(EXECUTOR)
    );
    assert_eq!(settled.workspace.host_cwd, expected);
    assert!(settled.llm_connection_id.is_none());
    assert_eq!(
        runtime
            .block_on(client.connection_catalog(ConnectionCatalogQueryInput::Start))
            .unwrap(),
        connections,
        "creation and execution cannot synthesize a model connection or default"
    );
    tui.close_terminal();
    tui.finish();
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
