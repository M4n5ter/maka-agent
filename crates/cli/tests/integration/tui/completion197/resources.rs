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
    plugin::{self, RemoteBinding, RemoteKind, RemoteRequest, RemoteResult},
};

const PACKAGE: &str = "example.completion197";
const SESSION: &str = "completion-plugin";
const PROOF: &str = "PLUGIN_CONTEXT_197 真实🦀";

async fn install(client: &Client, parent: &std::path::Path) -> plugin::InputResourceProjection {
    let package = parent.join("context-plugin");
    std::fs::create_dir(&package).unwrap();
    std::fs::write(package.join("host.mjs"), include_str!("context.mjs")).unwrap();
    std::fs::write(
        package.join("maka.extension.json"),
        json!({"schemaVersion":1,"id":PACKAGE,
        "runtime":{"entry":"host.mjs","sdkVersion":3,"vm":"dedicated"}})
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
        "entry":{"id":"completion-fixture","packageId":PACKAGE}}]}),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut cursor = None;
            loop {
                let plugin::QueryResult::InputResources(page) = client
                    .plugin_query(plugin::Query {
                        view: plugin::View::InputResources,
                        root_id: Some(maka_runtime::scope::Scope::Session(SESSION.into())),
                        cursor,
                        limit: Some(32),
                    })
                    .await
                    .unwrap()
                else {
                    panic!("input resource catalog")
                };
                if let Some(provider) = page
                    .items
                    .into_iter()
                    .find(|entry| entry.provider == PACKAGE)
                {
                    return provider;
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("installed provider did not publish")
}

async fn stats(client: &Client) -> Value {
    let binding = RemoteBinding::Package {
        package_id: PACKAGE.into(),
        method: "stats".into(),
        session_id: None,
    };
    let RemoteResult::Bound {
        target,
        handler: RemoteKind::Method,
    } = client
        .plugin_remote(RemoteRequest::Bind {
            binding: binding.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("stats method")
    };
    let RemoteResult::Document { document } = client
        .plugin_remote(RemoteRequest::OpenDocument)
        .await
        .unwrap()
    else {
        panic!("stats document")
    };
    let result = client
        .plugin_remote(RemoteRequest::Call {
            binding,
            target,
            document,
            input: Value::Null,
        })
        .await;
    assert!(matches!(
        client
            .plugin_remote(RemoteRequest::CloseDocument { document })
            .await
            .unwrap(),
        RemoteResult::Closed
    ));
    let RemoteResult::Value { value } = result.unwrap() else {
        panic!("stats value")
    };
    value
}

async fn await_stats(client: &Client, predicate: impl Fn(&Value) -> bool) -> Value {
    // This deadline is shorter than the provider's ten-second query timeout:
    // only actual UI cancellation can satisfy the post-Esc pending=0 check.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = stats(client).await;
            if predicate(&value) {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("independent provider state did not settle")
}

fn open_source(tui: &mut Pty, keyboard: bool) {
    typed(tui, "@");
    tui.wait_until(|screen| composer_has(screen, "@") && heading(screen, "Workspace"));
    if keyboard {
        tui.send(b"\x1b[Z");
        tui.wait_for("Added context");
        tui.send(b"\x1b[B\x1b[B\r");
        tui.wait_until(|screen| heading(screen, "Plugins"));
    } else {
        category(tui, "Workspace", "Plugins");
    }
    tui.wait_until(|screen| screen.contains("PTY context source") && candidates_settled(screen));
    tui.click_text("PTY context source");
    tui.wait_until(|screen| {
        screen.contains("Captured plugin evidence") && candidates_settled(screen)
    });
    no_palette(tui);
}

#[test]
fn javascript_composer_resources_cancel_readonly_lookup_and_prepare_exact_source() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("plugin-proof.txt"), PROOF).unwrap();
    let mut host = start(directory.path().join("root"));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (client, model) = runtime.block_on(model(
        &host.root,
        &workspace,
        SESSION,
        "Completion plugin",
        "Plugin captured context accepted.",
        calls.clone(),
    ));
    let provider = runtime.block_on(install(&client, directory.path()));
    let mut tui = Pty::spawn(&["--root", host.root.to_str().unwrap()]);
    open_session(&mut tui, "Completion plugin");
    open_source(&mut tui, true);
    let before = runtime.block_on(stats(&client));
    assert!(before["queries"].as_u64().unwrap() > 0);
    assert_eq!(before["resolved"], 0);
    assert_eq!(
        before["prepared"], 0,
        "typing and candidate rendering cannot prepare a turn"
    );
    typed(&mut tui, "slow");
    tui.wait_until(|screen| composer_has(screen, "@slow"));
    let pending = runtime.block_on(await_stats(&client, |stats| stats["pending"] == 1));
    assert_eq!(pending["prepared"], 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    tui.send(b"\x1b");
    let cancelled = runtime.block_on(await_stats(&client, |stats| {
        stats["pending"] == 0 && stats["cancelled"].as_u64().unwrap() >= 1
    }));
    assert_eq!(cancelled["prepared"], 0);
    assert_eq!(cancelled["resolved"], 0);
    tui.wait_until(|screen| composer_has(screen, "@slow") && !heading(screen, "Plugins"));
    // Local editing remains usable after cancelling the outstanding Host read.
    tui.send(b"\x01\x7f");
    tui.wait_for("Message…");
    tui.click_page_text("Message…");
    open_source(&mut tui, false);
    tui.click_text("Captured plugin evidence");
    tui.wait_until(|screen| {
        composer_has(screen, "@Captured plugin evidence") && !heading(screen, "Plugins")
    });
    let resolved = runtime.block_on(stats(&client));
    assert_eq!(resolved["resolved"], 1);
    assert_eq!(resolved["prepared"], 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    tui.click_last_text("@Captured plugin evidence");
    tui.send(b"\r");
    tui.wait_for("Plugin captured context accepted.");
    no_palette(&tui);
    let body = runtime.block_on(model).unwrap();
    assert!(body.to_string().contains(PROOF));
    assert!(body.to_string().contains("PREPARED_PLUGIN_SOURCE_197"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let original = runtime.block_on(source(&client, SESSION));
    assert_eq!(original.input_selections[PACKAGE], ["proof:197"]);
    let expected = maka_runtime::input::SelectionSource {
        provider: PACKAGE.into(),
        package_id: PACKAGE.into(),
        entry_id: provider.target.entry_id,
        activation: provider.target.activation,
        registration: provider.target.registration,
        session_id: SESSION.into(),
    };
    assert_eq!(
        original.input_selection_sources.as_slice(),
        std::slice::from_ref(&expected)
    );
    assert!(
        original
            .content
            .quotes
            .unwrap()
            .iter()
            .any(|quote| quote.text == PROOF)
    );
    assert!(
        !original.content.text.contains("PREPARED_PLUGIN_SOURCE_197"),
        "Host sources retain the original input"
    );
    let prepared = runtime.block_on(stats(&client));
    assert_eq!(prepared["prepared"], 1);
    assert_eq!(prepared["pending"], 0);
    assert_eq!(
        prepared["sources"],
        serde_json::to_value(vec![expected]).unwrap()
    );
    tui.close_terminal();
    tui.finish();
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
