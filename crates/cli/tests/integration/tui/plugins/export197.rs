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
use maka_protocol::plugin::PackageInstall;

#[test]
fn installed_package_export_uses_normal_pages_reviewed_bytes_and_preserves_unknown_target() {
    // Keep reviewed Host paths fully visible in the fixed-width terminal.
    let directory = tempfile::tempdir_in(std::fs::canonicalize("/tmp").unwrap()).unwrap();
    let source = package(
        directory.path(),
        "export.board",
        "Export board",
        "export default function() {}",
        ("notes.txt", "original"),
    );
    let mut host = host(directory.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        let client = support::client(&host.root).await;
        let preview = client
            .plugin_package_preview(source.to_str().unwrap().into())
            .await
            .unwrap();
        client
            .plugin_package_install(PackageInstall {
                source_path: preview.source_path,
                source_digest: Some(preview.package.content_digest),
                expected: Some(preview.expected),
            })
            .await
            .unwrap();
        client
    });
    let proxy = runtime.block_on(super::super::recovery::LostReply::package_export(
        &host.root,
        directory.path(),
    ));
    let target = directory.path().join("exported.maka-extension");
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    tui.wait_for("Settings");
    tui.click_text("Settings");
    tui.wait_for("Plugins");
    tui.click_page_text("Plugins");
    tui.wait_for("Installed packages");
    tui.click_page_text("Export board");
    tui.wait_for("Export package");
    tui.click_page_text("Export package");
    tui.wait_for("Installed package on Host");
    field(
        &mut tui,
        "New bundle file on Host",
        target.to_str().unwrap(),
    );
    tui.click_page_text("Export package");
    tui.wait_for("Confirm");
    tui.send(b"\r");
    tui.wait_until(|screen| !screen.contains("Confirm"));
    assert!(!target.exists(), "opening review defaults to cancellation");
    assert!(proxy.requests().is_empty());
    tui.click_page_text("Export package");
    tui.wait_for("Confirm");
    let latest_digest = runtime.block_on(async {
        std::fs::write(source.join("notes.txt"), "installed replacement").unwrap();
        let preview = client
            .plugin_package_preview(source.to_str().unwrap().into())
            .await
            .unwrap();
        let digest = preview.package.content_digest.clone();
        client
            .plugin_package_install(PackageInstall {
                source_path: preview.source_path,
                source_digest: Some(digest.clone()),
                expected: Some(preview.expected),
            })
            .await
            .unwrap();
        digest
    });
    tui.click_last_text("Confirm");
    tui.wait_until(|screen| {
        screen.contains("OperationConflict: Installed package changed since")
            && screen
                .lines()
                .any(|line| line.trim_end().ends_with("review"))
            && !screen.contains("Confirm")
    });
    assert!(
        !target.exists(),
        "a stale review cannot publish another installed version"
    );
    let stale = proxy.requests();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["extensionId"], "export.board");
    assert_eq!(stale[0]["targetPath"], target.to_str().unwrap());
    assert_ne!(stale[0]["expected"]["contentDigest"], latest_digest);
    // Changing the original source no longer changes the installed bytes.
    std::fs::write(source.join("notes.txt"), "later uninstalled source edit").unwrap();
    tui.wait_until(|screen| {
        !screen.contains("Contacting Host…")
            && screen.contains(latest_digest.strip_prefix("sha256-").unwrap())
    });
    tui.click_page_text("Export package");
    tui.wait_for("Confirm");
    tui.click_last_text("Confirm");
    tui.wait_for("This export is unconfirmed.");
    tui.wait_for("connection failed");
    assert_eq!(
        maka_plugins::package::Package::read_from(&target)
            .unwrap()
            .digest(),
        latest_digest
    );
    assert_eq!(proxy.requests().len(), 2);
    tui.close_terminal();
    tui.finish();
    let mut reopened = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    reopened.wait_for("This export is unconfirmed.");
    reopened.wait_for(target.to_str().unwrap());
    reopened.click_page_text("Export package");
    assert_eq!(
        proxy.requests().len(),
        2,
        "unknown original export cannot be repeated on reopen"
    );
    reopened.close_terminal();
    reopened.finish();
    drop(proxy);
    runtime.block_on(async {
        let preview = client.plugin_package_preview(target.to_str().unwrap().into()).await.unwrap();
        assert_eq!(preview.package.content_digest, latest_digest);
        let output = client.plugin_package_export(maka_protocol::plugin::PackageExport {
            extension_id: "export.board".into(), target_path: target.to_str().unwrap().into(),
            expected: preview.expected,
        }).await;
        assert!(matches!(output, Err(maka_client::RequestFailure::Rejected(maka_client::ClientError::Rejected(error)))
            if error.code == maka_protocol::OperationErrorCode::OperationConflict));
    });
    assert_eq!(
        maka_plugins::package::Package::read_from(&target)
            .unwrap()
            .digest(),
        latest_digest
    );
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
