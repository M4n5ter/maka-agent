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
    project::Mutation,
    session::{WorkspaceTarget, decode_session_create_input},
};
use serde_json::json;

fn start(root: std::path::PathBuf) -> super::super::candidate::CandidateFixture {
    let mut host = super::super::candidate::CandidateFixture::new(root);
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
    host
}
fn paste_field(tui: &mut Pty, label: &str, text: &str) {
    tui.wait_for(label);
    let screen = tui.screen.snapshot().unwrap().screen;
    let (column, row) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(label)
                .map(|offset| (line[..offset].width() + 1, row + 2))
        })
        .expect("field label is visible");
    tui.send(
        format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m\x01\x1b[200~{text}\x1b[201~")
            .as_bytes(),
    );
    tui.wait_for(text);
}
fn open_import(tui: &mut Pty) {
    tui.wait_until(|screen| {
        screen.contains("Import session bundle") || screen.contains("Session transfer")
    });
    let screen = tui.screen.snapshot().unwrap().screen;
    tui.click_text(if screen.contains("Session transfer") {
        "Session transfer"
    } else {
        "Import session bundle"
    });
}

/// Real page entry -> real Host -> actual file -> a second Host's durable
/// receipt. The relay loses only a genuine accepted reply and never fabricates
/// a success. No command palette participates in this workflow.
#[test]
fn native_bundle_pages_export_import_and_recover_original_receipt_without_replay() {
    // Keep reviewed Host paths fully visible in the fixed-width terminal.
    let directory = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap()).unwrap();
    let source_work = directory.path().join("source-work");
    let target_work = directory.path().join("target-work");
    let project_work = directory.path().join("project-work");
    for path in [&source_work, &target_work, &project_work] {
        std::fs::create_dir(path).unwrap();
    }
    let mut source = start(directory.path().join("source-root"));
    let mut target = start(directory.path().join("target-root"));
    let mut project_target = start(directory.path().join("project-root"));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (source_client, target_client, project_client, project) = runtime.block_on(async {
        let source_client = support::model_client(&source.root, "http://127.0.0.1:9/v1").await;
        source_client.create_session(decode_session_create_input(&json!({
            "sessionId":"bundle-source","name":"Bundle source",
            "workspace":{"kind":"host_path","path":source_work},"modelTarget":{"kind":"default"}
        })).unwrap()).await.unwrap();
        let target_client = support::model_client(&target.root, "http://127.0.0.1:9/v1").await;
        let project_client =
            support::model_client(&project_target.root, "http://127.0.0.1:9/v1").await;
        let project = project_client
            .mutate_project(Mutation::Register {
                path: project_work.to_str().unwrap().into(),
                prefer: Some(true),
            })
            .await
            .unwrap();
        let project = project_client
            .mutate_project(Mutation::Rename {
                project_id: project.id,
                name: "Bundle destination project".into(),
            })
            .await
            .unwrap();
        (source_client, target_client, project_client, project)
    });
    let exported_path = directory.path().join("session.maka-session");
    let mut tui = Pty::spawn(&["--root", source.root.to_str().unwrap()]);
    tui.wait_for("Bundle source");
    tui.click_text("Bundle source");
    tui.wait_for("Message…");
    tui.send(b"unsubmitted source draft");
    tui.wait_for("unsubmitted source draft");
    tui.open_header_actions();
    tui.wait_for("Export session bundle");
    tui.click_text("Export session bundle");
    tui.wait_for("including this session and all subtasks");
    paste_field(
        &mut tui,
        "New bundle file on Host",
        exported_path.to_str().unwrap(),
    );
    tui.resize(52, 24);
    tui.wait_for("Review transfer");
    tui.resize(120, 40);
    // resize invalidates Frames::ready; wait for Ratatui's complete cleared
    // frame before finding pointer coordinates in the new layout.
    tui.wait_for("Review transfer");
    tui.click_text("Review transfer");
    tui.wait_for("Confirmed subtree digest");
    tui.send(b"\r");
    tui.wait_until(|screen| !screen.contains("Confirmed subtree digest"));
    assert!(!exported_path.exists(), "default Close cannot publish");
    tui.open_header_actions();
    tui.wait_for("Session transfer");
    tui.click_text("Session transfer");
    tui.wait_for("Confirmed subtree digest");
    tui.click_last_text("Export");
    tui.wait_for("Host exported 1 sessions");
    let bytes = std::fs::read(&exported_path).unwrap();
    assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
    tui.send(b"\x1b");
    tui.wait_until(|screen| {
        !screen.contains("Host exported 1 sessions") && screen.contains("unsubmitted source draft")
    });
    tui.close_terminal();
    tui.finish();
    let project_source = directory.path().join("project.maka-session");
    std::fs::write(&project_source, &bytes).unwrap();

    let proxy = runtime.block_on(recovery::LostReply::bundle_import(
        &target.root,
        directory.path(),
    ));
    let mut tui = Pty::spawn(&["--root", target.root.to_str().unwrap()]);
    tui.wait_for("No sessions yet");
    open_import(&mut tui);
    paste_field(
        &mut tui,
        "Bundle file on Host",
        exported_path.to_str().unwrap(),
    );
    paste_field(
        &mut tui,
        "Destination directory on Host",
        target_work.to_str().unwrap(),
    );
    tui.click_text("Review transfer");
    tui.wait_for("Resolved directory on Host");
    tui.wait_for(target_work.to_str().unwrap());
    tui.click_last_text("Import");
    tui.wait_for("The Host may have completed this transfer.");
    tui.wait_for("connection failed");
    assert_eq!(proxy.requests().len(), 1);
    tui.close_terminal();
    tui.finish();
    runtime.block_on(async {
        let imported = target_client
            .session("bundle-source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(imported.name, "Bundle source");
        assert_eq!(imported.workspace.host_cwd, target_work.to_str().unwrap());
        let page = target_client
            .connection_catalog(ConnectionCatalogQueryInput::Start)
            .await
            .unwrap();
        target_client
            .request(
                Operation::ConnectionCatalogSetDefaultTarget,
                json!({
                    "expectedCatalogRevision":page["revision"],"target":null,
                }),
            )
            .await
            .unwrap();
    });
    std::fs::remove_file(&exported_path).unwrap();
    let mut reopened = Pty::spawn(&["--root", target.root.to_str().unwrap()]);
    reopened.wait_for("Bundle source");
    open_import(&mut reopened);
    reopened.wait_for("The Host may have completed this transfer.");
    reopened.click_text("Check result");
    reopened.wait_for("Original receipt confirms 1 imported sessions.");
    reopened.wait_for("artifact count.");
    assert_eq!(
        proxy.requests().len(),
        1,
        "receipt recovery cannot re-import a changed or missing source"
    );
    reopened.click_text("Open session");
    reopened.wait_for("Message…");
    reopened.close_terminal();
    reopened.finish();
    drop(proxy);

    let mut tui = Pty::spawn(&["--root", project_target.root.to_str().unwrap()]);
    tui.wait_for("No sessions yet");
    open_import(&mut tui);
    paste_field(
        &mut tui,
        "Bundle file on Host",
        project_source.to_str().unwrap(),
    );
    tui.click_text("Choose project");
    tui.wait_for("Bundle destination project");
    tui.click_text("Bundle destination project");
    tui.wait_for("Review transfer");
    tui.click_text("Review transfer");
    tui.wait_for("Resolved directory on Host");
    tui.wait_for(project_work.to_str().unwrap());
    tui.click_last_text("Import");
    tui.wait_for("Host imported 1 sessions");
    runtime.block_on(async {
        let imported = project_client
            .session("bundle-source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            imported.workspace.target,
            WorkspaceTarget::Project {
                project_id: project.id
            }
        );
        assert_eq!(imported.workspace.host_cwd, project_work.to_str().unwrap());
        let original = source_client
            .session("bundle-source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.workspace.host_cwd, source_work.to_str().unwrap());
    });
    tui.close_terminal();
    tui.finish();
    source_client.disconnect();
    target_client.disconnect();
    project_client.disconnect();
    for host in [&mut source, &mut target, &mut project_target] {
        host.retire_registered();
        assert!(host.wait_for_exit().success());
    }
}
