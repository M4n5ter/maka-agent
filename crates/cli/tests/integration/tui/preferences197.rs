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
use maka_protocol::configuration::{
    CredentialState, CredentialVaultQueryResult, policy::network_update,
};

#[test]
fn preferences197_normal_settings_save_proxy_auth_test_and_delete_without_secret_persistence() {
    let directory = tempfile::tempdir().unwrap();
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
    let (client, port, probe) = runtime.block_on(async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let client = support::client(&host.root).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let probe = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(20), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0 && bytes.len() < 16_384);
                bytes.extend_from_slice(&buffer[..read]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with("GET http://example.test/probe "));
            assert!(request.lines().any(|line| line.split_once(':').is_some_and(
                |(name, value)| name.eq_ignore_ascii_case("proxy-authorization")
                    && value.trim() == "Basic dGVzdC11c2VyOnByaXZhdGUtcHJveHktcGFzc3dvcmQ="
            )));
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        (client, port, probe)
    });
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    tui.resize(110, 40);
    tui.wait_for("Settings");
    tui.click_text("Settings");
    tui.wait_for("Customize theme");
    tui.click_page_text("Host");
    tui.wait_for("Network proxy");
    tui.wait_for("Connected");
    tui.click_text("Network proxy");
    tui.wait_for("Use proxy");
    tui.click_text("Use proxy");
    tui.wait_until(|screen| {
        screen
            .lines()
            .any(|line| line.contains("Use proxy") && line.contains("Enabled"))
    });
    replace(&mut tui, "Port", &port.to_string());
    tui.click_text("Authentication");
    tui.wait_for("Username");
    replace(&mut tui, "Username", "test-user");
    tui.click_text("Keep existing");
    tui.wait_for("Replace");
    tui.click_text("Replace");
    tui.wait_for("New password");
    replace_secret(&mut tui, "New password", "private-proxy-password");
    replace(&mut tui, "Test URL", "http://example.test/probe");
    tui.click_last_text("Save");
    tui.wait_for("Saved.");
    let saved = runtime.block_on(client.runtime_policy()).unwrap();
    assert!(saved.policy.network_proxy.enabled);
    assert!(saved.policy.network_proxy.auth_enabled);
    assert_eq!(saved.policy.network_proxy.port, port);
    assert_eq!(saved.policy.network_proxy.username, "test-user");
    tui.click_text("Test saved settings");
    tui.wait_for("Connection succeeded");
    runtime.block_on(probe).unwrap();
    tui.click_text("Authentication");
    tui.wait_until(|screen| !screen.contains("Username"));
    tui.click_last_text("Save");
    tui.wait_for("Saved.");
    let CredentialVaultQueryResult::Status { status } = runtime
        .block_on(client.credential_status(network_update::locator()))
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(status.state, CredentialState::Absent));
    assert!(
        !runtime
            .block_on(client.runtime_policy())
            .unwrap()
            .policy
            .network_proxy
            .auth_enabled
    );
    tui.click_last_text("Cancel");
    tui.close_terminal();
    tui.finish();
    let checkpoint = directory
        .path()
        .join("tui-state")
        .join(&client.identity.root_id)
        .join("default/state.json");
    let saved = std::fs::read_to_string(checkpoint).unwrap();
    assert!(!String::from_utf8_lossy(&tui.output).contains("private-proxy-password"));
    assert!(!saved.contains("private-proxy-password"));
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
fn replace(tui: &mut Pty, label: &str, value: &str) {
    replace_presented(tui, label, value, value);
}
fn replace_secret(tui: &mut Pty, label: &str, value: &str) {
    replace_presented(tui, label, value, &"*".repeat(value.width()));
}
fn replace_presented(tui: &mut Pty, label: &str, value: &str, expected: &str) {
    tui.click_text(label);
    tui.send(b"\x01");
    tui.send(value.as_bytes());
    // A completed earlier frame is insufficient: typing can still be queued,
    // and validation may change the form before the next pointer action.
    tui.wait_until(|screen| {
        screen.lines().any(|line| {
            line.split_once(label)
                .is_some_and(|(_, field)| field.trim().trim_end_matches('│').trim() == expected)
        })
    });
}

#[test]
fn preferences197_normal_connection_menu_edits_provider_headers_and_request_overlay_with_cas() {
    use maka_protocol::configuration::{
        ConnectionCatalogQueryInput,
        headers::{RequestHeaderUpdate, RequestHeadersQueryResult, RequestHeadersReplace},
    };
    let directory = tempfile::tempdir().unwrap();
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
    let client = runtime.block_on(support::model_client(&host.root, "http://127.0.0.1:9/v1"));
    let catalog = || {
        runtime
            .block_on(client.connection_catalog(ConnectionCatalogQueryInput::Start))
            .unwrap()
    };
    let id = catalog()["items"][0]["connectionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    tui.resize(110, 40);
    tui.wait_for("Settings");
    tui.click_text("Settings");
    tui.wait_for("Models");
    tui.click_text("Models");
    tui.wait_for("Model connections");
    tui.click_text("Model connections");
    open_connection(&mut tui, "Provider settings");
    tui.wait_for("Base URL");
    replace(&mut tui, "Base URL", "http://127.0.0.1:9/v2");
    tui.click_last_text("Save");
    tui.wait_until(|screen| !screen.contains("Cancel"));
    assert_eq!(
        catalog()["items"][0]["configuration"]["baseUrl"],
        "http://127.0.0.1:9/v2"
    );

    open_connection(&mut tui, "Request headers");
    tui.wait_for("Add header");
    tui.click_text("Add header");
    tui.wait_for("Header name");
    replace(&mut tui, "Header name", "X-Trace");
    replace_secret(&mut tui, "New value", "private-header-value");
    tui.click_last_text("Save");
    tui.wait_for("Saved.");
    let RequestHeadersQueryResult::Found { names, basis } =
        runtime.block_on(client.request_headers(&id)).unwrap()
    else {
        panic!()
    };
    assert_eq!(names, ["X-Trace"]);
    tui.click_text("Keep existing");
    tui.wait_for("Replace");
    tui.click_text("Replace");
    tui.wait_for("New value");
    replace_secret(&mut tui, "New value", "uncommitted-private-value");
    runtime
        .block_on(client.replace_request_headers(&RequestHeadersReplace {
            expected: basis,
            headers: vec![RequestHeaderUpdate {
                name: "X-External".into(),
                value: Some("external-private-value".into()),
            }],
        }))
        .unwrap();
    tui.click_last_text("Save");
    tui.wait_for("Settings changed");
    let RequestHeadersQueryResult::Found { names, .. } =
        runtime.block_on(client.request_headers(&id)).unwrap()
    else {
        panic!()
    };
    assert_eq!(names, ["X-External"]);
    tui.click_text("Reload and discard edits");
    tui.wait_for("X-External");
    tui.click_text("Keep existing");
    tui.wait_for("Delete");
    tui.click_text("Delete");
    tui.wait_until(|screen| !screen.contains("Header name"));
    tui.click_last_text("Save");
    tui.wait_for("Saved.");
    let RequestHeadersQueryResult::Found { names, .. } =
        runtime.block_on(client.request_headers(&id)).unwrap()
    else {
        panic!()
    };
    assert!(names.is_empty());
    tui.click_last_text("Cancel");

    open_connection(&mut tui, "Advanced request body");
    tui.wait_for("JSON overrides");
    replace(&mut tui, "JSON overrides", "{\"temperature\":0.3}");
    tui.click_last_text("Save");
    tui.wait_until(|screen| !screen.contains("Cancel"));
    assert_eq!(
        catalog()["items"][0]["requestBodyOverlay"]["temperature"],
        0.3
    );
    open_connection(&mut tui, "Advanced request body");
    tui.wait_for("Clear overrides");
    tui.click_text("Clear overrides");
    tui.wait_until(|screen| !screen.contains("temperature"));
    tui.click_last_text("Save");
    tui.wait_until(|screen| !screen.contains("Cancel"));
    assert!(catalog()["items"][0].get("requestBodyOverlay").is_none());
    tui.close_terminal();
    tui.finish();
    let saved = std::fs::read_to_string(
        directory
            .path()
            .join("tui-state")
            .join(&client.identity.root_id)
            .join("default/state.json"),
    )
    .unwrap();
    for secret in [
        "private-header-value",
        "uncommitted-private-value",
        "external-private-value",
    ] {
        assert!(!String::from_utf8_lossy(&tui.output).contains(secret));
        assert!(!saved.contains(secret));
    }
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
fn open_connection(tui: &mut Pty, label: &str) {
    tui.wait_for("TUI fixture");
    tui.click_text("TUI fixture");
    tui.open_header_actions();
    tui.wait_for(label);
    tui.click_text(label);
}
