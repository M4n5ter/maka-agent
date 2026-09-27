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
    resource::{ControllerIdentity, ResourceQueryInput, ResourceQueryResult},
    session::decode_session_create_input,
};
use maka_runtime::shell_result::{ShellMode, ShellOutput, ShellStatus};
use serde_json::json;

#[test]
fn session_terminal_page_runs_real_pipes_and_pty_and_releases_native_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
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
    let client=runtime.block_on(async {
        let client=support::model_client(&host.root,"http://127.0.0.1:9/v1").await;
        client.create_session(decode_session_create_input(&json!({"sessionId":"resources197","name":"Resource work","workspace":{"kind":"host_path","path":workspace},"sandboxMode":"danger-full-access","modelTarget":{"kind":"default"}})).unwrap()).await.unwrap();client
    });
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    tui.resize(120, 45);
    tui.wait_for("Resource work");
    tui.click_text("Resource work");
    tui.wait_for("fixture-model");
    tui.open_header_actions();
    tui.wait_for("Terminal");
    tui.click_text("Terminal");
    tui.wait_for("New terminal");
    tui.click_text("Run command");
    tui.wait_for("Command");
    tui.click_text("Command");
    tui.send(b"\x1b[200~printf 'resource-pipes-197\n'; pwd; printf 'stderr-197\n' >&2\x1b[201~");
    tui.click_last_text("Run");
    tui.wait_for("Completed");
    let page = runtime
        .block_on(client.resource_query(ResourceQueryInput::ListStart {
            session_id: "resources197".into(),
        }))
        .unwrap();
    let ResourceQueryResult::Page { resources, .. } = page else {
        panic!("resource page")
    };
    let pipes = resources
        .iter()
        .find(|row| row.result.mode == ShellMode::Pipes)
        .expect("native user pipes resource");
    assert_eq!(pipes.result.status, ShellStatus::Completed);
    assert_eq!(
        pipes.result.cwd,
        workspace.canonicalize().unwrap().to_str().unwrap()
    );
    let Some(ShellOutput::Pipes { stdout, stderr, .. }) = &pipes.result.output else {
        panic!("pipes projection")
    };
    assert!(stdout.contains("resource-pipes-197"));
    assert!(stdout.contains(workspace.to_str().unwrap()));
    assert!(stderr.contains("stderr-197"));
    tui.click_text("New terminal");
    capture_terminal(&mut tui);
    // Input travels through the actual controller into the shell, not an Agent tool.
    tui.send(b"printf 'native-%s-%s\n' pty 197\r");
    wait_terminal(&mut tui, "native-pty-197");
    let page = runtime
        .block_on(client.resource_query(ResourceQueryInput::ListStart {
            session_id: "resources197".into(),
        }))
        .unwrap();
    let ResourceQueryResult::Page { resources, .. } = page else {
        panic!("resource page")
    };
    let pty = resources
        .iter()
        .find(|row| row.result.mode == ShellMode::Pty)
        .expect("native PTY resource");
    let reference = pty.result.resource_ref.clone();
    let conflict = runtime.block_on(client.resource_controller_acquire(ControllerIdentity {
        session_id: "resources197".into(),
        resource_ref: reference.clone(),
        controller_id: "second-controller".into(),
    }));
    assert!(
        matches!(conflict,Err(maka_client::RequestFailure::Rejected(maka_client::ClientError::Rejected(error))) if error.code==maka_protocol::OperationErrorCode::OperationConflict)
    );
    // The Host keeps the screen while its bounded raw tail starts inside an OSC.
    // Reattaching must retain the early screen marker and never render hidden payload.
    tui.send(b"stty -echo; printf '\\033[2J\\033[Hcanonical-anchor-197'; i=0; while test \"$i\" -lt 8; do printf '\\033]0;'; head -c 15000 /dev/zero | tr '\\000' X; printf '\\007'; i=$((i+1)); done; printf '\\033[2;1H%s-%s\\n' canonical-tail 197; stty echo\r");
    wait_terminal(&mut tui, "canonical-tail-197");
    assert!(
        tui.screen
            .snapshot()
            .unwrap()
            .screen
            .contains("canonical-anchor-197")
    );
    // The mode transition fences Reconnect; an old ready frame from before the
    // click cannot satisfy the next capture while its old lease is retiring.
    tui.click_text("Reconnect terminal");
    wait_terminal(&mut tui, "Terminal screen");
    capture_terminal(&mut tui);
    tui.send(b"printf '\\033[3;1H%s-%s\\n' reconnected 197\r");
    wait_terminal(&mut tui, "reconnected-197");
    assert!(
        tui.screen
            .snapshot()
            .unwrap()
            .screen
            .contains("canonical-anchor-197")
    );
    tui.resize(90, 35);
    tui.send(b"\x1d");
    wait_terminal(&mut tui, "Stop process");
    tui.click_text("Stop process");
    wait_terminal(&mut tui, "Stop this process");
    tui.click_text("Stop process");
    wait_terminal(&mut tui, "Stopped");
    let found = runtime
        .block_on(client.resource_query(ResourceQueryInput::Get {
            session_id: "resources197".into(),
            resource_ref: reference,
        }))
        .unwrap();
    let ResourceQueryResult::Resource {
        resource: Some(ended),
        ..
    } = found
    else {
        panic!("ended native PTY")
    };
    assert!(!matches!(
        ended.result.status,
        ShellStatus::Starting | ShellStatus::Running
    ));
    tui.close_terminal();
    tui.finish();
    let checkpoint = std::fs::read_to_string(
        directory
            .path()
            .join("tui-state")
            .join(&client.identity.root_id)
            .join("default/state.json"),
    )
    .unwrap();
    assert!(!checkpoint.contains("native-pty-197"));
    assert!(!checkpoint.contains("resource-pipes-197"));
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}

#[track_caller]
fn capture_terminal(tui: &mut Pty) {
    // Canonical output can arrive before the controller lease. Wait for the
    // visible readiness state used by capture, including after a reconnect.
    wait_terminal_condition(tui, |screen| {
        screen.contains("Type in terminal") && !screen.contains("Connecting terminal")
    });
    tui.click_text("Type in terminal");
    wait_terminal(tui, "Ctrl+]");
}

#[track_caller]
fn wait_terminal(tui: &mut Pty, text: &str) {
    wait_terminal_condition(tui, |screen| screen.contains(text));
}

#[track_caller]
fn wait_terminal_condition(tui: &mut Pty, ready: impl Fn(&str) -> bool) {
    tui.wait_until(|screen| ready(screen) || screen.contains("connection failed"));
    if tui
        .screen
        .snapshot()
        .unwrap()
        .screen
        .contains("connection failed")
    {
        // The ordinary details surface already contains the actual ClientError.
        // Expose it on this synthetic fixture's failure frame without tracing
        // arbitrary controller input, notification payloads or Host credentials.
        tui.click_text("Review details");
        tui.wait_until(|screen| {
            screen.contains("Host connection closed:")
                || screen.contains("Invalid Host protocol:")
                || screen.contains("Host request timed out")
                || screen.contains("Host identity, compatibility")
        });
        panic!("Terminal connection failed; the final frame contains the Client diagnostic");
    }
}
