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

use maka_computer_use::{Driver, Session, facade};
use maka_js_runtime::{CellContext, CellLimits, CellOutput, CellResult, CellStore, repl::Repl};
use maka_plugins::computer::BrowserConnection;
use maka_runtime::tools::{ToolExecutor, ToolFuture};
use serde_json::{Value, json};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

struct Bridge(Arc<Mutex<Session>>);
impl ToolExecutor for Bridge {
    fn names(&self) -> Vec<String> {
        vec!["cua".into()]
    }
    fn invoke(&self, _: String, input: Value, cancellation: CancellationToken) -> ToolFuture {
        let session = self.0.clone();
        Box::pin(async move {
            session
                .lock()
                .await
                .invoke(
                    serde_json::from_value(input).unwrap(),
                    "browser-fixture",
                    &cancellation,
                )
                .await
        })
    }
}
async fn evaluate(repl: &Repl, bridge: Arc<Bridge>, source: String) -> (bool, Vec<CellOutput>) {
    let context = CellContext::new(CellStore::default(), 16 * 1024 * 1024, vec![]);
    let result = repl
        .evaluate(
            source,
            bridge,
            CancellationToken::new(),
            context.clone(),
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    let ok = matches!(result, CellResult::Success { .. });
    if !ok {
        eprintln!("{result:?}");
    }
    (ok, context.take_output())
}
fn texts(output: &[CellOutput]) -> String {
    output
        .iter()
        .filter_map(|part| match part {
            CellOutput::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn index(state: &str, role: &str, name: &str) -> u64 {
    state
        .lines()
        .find(|line| line.contains(&format!(" {role} {name:?}")))
        .unwrap_or_else(|| panic!("missing {role} {name}: {state}"))
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires macOS Accessibility; compiles and operates a disposable native form"]
async fn native_form_retains_exact_ax_elements_and_verifies_submission() {
    let mut form = NativeForm::start(false).await;
    let window_id = form.window_id;
    let driver = Arc::new(Mutex::new(Driver::default()));
    let mut native = Session::new(driver.clone());
    let windows = native
        .invoke(
            serde_json::from_value(json!({"method":"listWindows","options":{}})).unwrap(),
            "browser-fixture",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let window = windows
        .as_array()
        .unwrap()
        .iter()
        .find(|window| window["id"] == window_id)
        .unwrap();
    let bridge = Arc::new(Bridge(Arc::new(Mutex::new(native))));
    let clipboard = clipboard_snapshot();
    let repl = Repl::new(
        facade(),
        CellLimits {
            max_value_bytes: 16 * 1024 * 1024,
            heap_bytes: 128 * 1024 * 1024,
            ..Default::default()
        },
    );
    let (ok, output) = evaluate(
        &repl,
        bridge.clone(),
        format!("const app=await cua.getApp({{windowId:{}}});", window["id"]),
    )
    .await;
    assert!(ok, "{}", texts(&output));
    let state = texts(&output);
    let name = index(&state, "AXTextField", "Name");
    let message = index(&state, "AXTextArea", "Message");
    let button = index(&state, "AXButton", "Submit fixture");
    let (ok,output)=evaluate(&repl,bridge.clone(),format!("await app.click({name}); await app.selectText({name}, 'hello', {{prefix:'hello '}}); await app.paste('world'); await app.setValue({message}, 'placeholder'); await app.click({message}); await app.selectText({message}, 'placeholder'); await app.paste('<b>你好</b>', {{format:'html'}}); await app.click({button}); await app.getAXState({{disableDiffing:true}});")).await;
    assert!(ok, "{}", texts(&output));
    let submitted = form.line().await;
    let submitted: Value = serde_json::from_str(&submitted).unwrap();
    assert!(
        submitted["submitted"] == "hello world / 你好",
        "native paste did not produce the requested text; rich={}, font={}",
        submitted["rich"],
        submitted["font"]
    );
    assert!(
        submitted["bold"] == true,
        "native HTML paste lost bold formatting; rich={}, font={}",
        submitted["rich"],
        submitted["font"]
    );
    assert!(
        texts(&output).contains("hello world / 你好"),
        "{}",
        texts(&output)
    );
    assert!(
        clipboard == clipboard_snapshot(),
        "clipboard contents were not restored (or changed externally during the test)"
    );
    let (ok, output) = evaluate(&repl, bridge.clone(), "await app.getScreenshot();".into()).await;
    assert!(ok, "native screenshot capture failed");
    assert!(
        output
            .iter()
            .any(|part| matches!(part, CellOutput::Media { .. }))
    );
    assert_eq!(form.command("move").await, "moved");
    let (ok, _) = evaluate(&repl, bridge.clone(), "await app.click([5,5]);".into()).await;
    assert!(
        !ok,
        "external window movement must reject old screenshot coordinates"
    );
    form.stop().await;
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        format!("await app.setValue({name}, 'wrong process');"),
    )
    .await;
    assert!(!ok);
    repl.close().await.unwrap();
    bridge.0.lock().await.close().await.unwrap();
    driver.lock().await.close().await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires macOS Accessibility; observes a background fixture without keyboard input"]
async fn native_observation_recovers_after_busy_and_preserves_external_edits() {
    let mut form = NativeForm::start(true).await;
    assert_eq!(form.command("activity").await, "inactive");
    let driver = Arc::new(Mutex::new(Driver::default()));
    let bridge = Arc::new(Bridge(Arc::new(Mutex::new(Session::new(driver.clone())))));
    let repl = Repl::new(
        facade(),
        CellLimits {
            max_value_bytes: 16 * 1024 * 1024,
            heap_bytes: 128 * 1024 * 1024,
            ..Default::default()
        },
    );
    let (ok, output) = evaluate(
        &repl,
        bridge.clone(),
        format!(
            "const app = await cua.getApp({{windowId:{}}}); await app.getAXStateAndScreenshot();",
            form.window_id
        ),
    )
    .await;
    assert!(ok, "{}", texts(&output));
    let name = index(&texts(&output), "AXTextField", "Name");

    // Hold the fixture's UI thread until the AX request times out, then
    // explicitly release it. No timing assumption or physical input is needed.
    assert_eq!(form.command("pause").await, "paused");
    let (ok, output) = evaluate(
        &repl,
        bridge.clone(),
        "try { await app.getAXState(); } catch (e) { nodeRepl.write(String(e)); }".into(),
    )
    .await;
    assert!(
        ok && texts(&output).contains("AX window unavailable"),
        "{}",
        texts(&output)
    );
    assert_eq!(form.command("resume").await, "resumed");

    let (ok, output) = evaluate(&repl, bridge.clone(), format!(
        "try {{ await app.setValue({name}, 'must not overwrite'); }} catch (e) {{ nodeRepl.write(String(e)); }}
         try {{ await app.click([5,5]); }} catch (e) {{ nodeRepl.write(String(e)); }}"
    )).await;
    assert!(
        ok && texts(&output).contains("stale_element")
            && texts(&output).contains("capture_required"),
        "{}",
        texts(&output)
    );
    let (ok, output) = evaluate(&repl, bridge.clone(), "await app.getAXState();".into()).await;
    assert!(
        ok && texts(&output).contains("hello hello"),
        "the same binding must recover without rebind: {}",
        texts(&output)
    );
    let name = index(&texts(&output), "AXTextField", "Name");

    assert_eq!(form.command("edit").await, "edited");
    let (ok, output) = evaluate(&repl, bridge.clone(), format!(
        "try {{ await app.selectText({name}, 'hello', {{prefix:'hello '}}); }} catch (e) {{ nodeRepl.write(String(e)); }}"
    )).await;
    assert!(
        ok && texts(&output).contains("text_not_found"),
        "{}",
        texts(&output)
    );
    let (ok, output) = evaluate(&repl, bridge.clone(), "await app.getAXState();".into()).await;
    assert!(
        ok && texts(&output).contains("externally edited"),
        "external edits must remain intact: {}",
        texts(&output)
    );

    assert_eq!(form.command("activity").await, "inactive");
    assert_eq!(form.command("close").await, "closed");
    assert!(
        form.app.try_wait().unwrap().is_none(),
        "the window closed, not the process"
    );
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        format!("await app.setValue({name}, 'closed window');"),
    )
    .await;
    assert!(
        !ok,
        "preserving a binding must not permit input to a closed window"
    );
    form.stop().await;
    repl.close().await.unwrap();
    bridge.0.lock().await.close().await.unwrap();
    driver.lock().await.close().await.unwrap();
}

#[cfg(target_os = "macos")]
struct NativeForm {
    app: tokio::process::Child,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    window_id: u64,
    _directory: tempfile::TempDir,
}

#[cfg(target_os = "macos")]
impl NativeForm {
    async fn start(background: bool) -> Self {
        assert!(
            unsafe { platform_macos::ax::bindings::AXIsProcessTrusted() },
            "native acceptance requires macOS Accessibility permission for the test host"
        );
        let directory = tempfile::tempdir().unwrap();
        let contents = directory.path().join("Maka Cua Fixture.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(contents.join("Info.plist"),r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>org.apache.maka.cua.fixture.FIXTURE_ID</string><key>CFBundleName</key><string>Maka Cua Fixture</string><key>CFBundleExecutable</key><string>fixture</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#.replace("FIXTURE_ID", &uuid::Uuid::new_v4().to_string())).unwrap();
        let executable = contents.join("MacOS/fixture");
        let compiled = tokio::process::Command::new("/usr/bin/swiftc")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/native-form.swift"
            ))
            .arg("-o")
            .arg(&executable)
            .output()
            .await
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let mut launch = tokio::process::Command::new(&executable);
        if background {
            launch.arg("--background");
        }
        let mut app = launch
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let lines = BufReader::new(app.stdout.take().unwrap()).lines();
        let mut fixture = Self {
            app,
            lines,
            window_id: 0,
            _directory: directory,
        };
        fixture.window_id = fixture
            .line()
            .await
            .strip_prefix("ready ")
            .unwrap()
            .parse()
            .unwrap();
        fixture
    }
    async fn line(&mut self) -> String {
        tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .expect("native fixture responds")
            .unwrap()
            .expect("native fixture remains alive")
    }
    async fn command(&mut self, command: &str) -> String {
        self.app
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{command}\n").as_bytes())
            .await
            .unwrap();
        self.line().await
    }
    async fn stop(&mut self) {
        self.app.kill().await.unwrap();
        self.app.wait().await.unwrap();
    }
}

#[cfg(target_os = "macos")]
fn clipboard_snapshot() -> Vec<Vec<(String, Vec<u8>)>> {
    objc2_app_kit::NSPasteboard::generalPasteboard()
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.types()
                        .iter()
                        .map(|kind| (kind.to_string(), item.dataForType(&kind).unwrap().to_vec()))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires Chrome; starts its own temporary headless profile and loopback fixture"]
async fn browser_form_uses_independent_repl_and_rejects_stale_document_and_tab() {
    let executable = std::env::var("MAKA_CUA_TEST_CHROME")
        .unwrap_or_else(|_| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    let profile = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let body = r#"<!doctype html><meta charset="utf-8"><title>Maka Cua fixture</title>
<label>Name <input id="name" value="hello hello"></label>
<label>Message <textarea id="message"></textarea></label>
<select aria-label="Color" id="color"><option value="red-id">Red</option><option value="blue-id">Blue</option></select>
<button onclick="document.getElementById('result').textContent=document.getElementById('name').value+' / '+document.getElementById('message').value+' / '+document.getElementById('color').value">Submit fixture</button>
<output id="result" aria-live="polite"></output>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    let mut chrome = tokio::process::Command::new(executable)
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "--disable-gpu",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg(&page)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(chrome.stderr.take().unwrap()).lines();
    let websocket = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(url) = line.strip_prefix("DevTools listening on ") {
                return url.to_owned();
            }
        }
        panic!("Chrome did not expose its test endpoint");
    })
    .await
    .unwrap();
    let logging =
        tokio::spawn(async move { while lines.next_line().await.ok().flatten().is_some() {} });
    let url = reqwest::Url::parse(&websocket).unwrap();
    let driver = Arc::new(Mutex::new(Driver::default()));
    let mut native = Session::new(driver.clone());
    native
        .configure_browsers(vec![BrowserConnection {
            id: "fixture".into(),
            endpoint: format!("http://127.0.0.1:{}", url.port().unwrap()),
        }])
        .unwrap();
    let session = Arc::new(Mutex::new(native));
    let bridge = Arc::new(Bridge(session.clone()));
    let repl = Repl::new(
        facade(),
        CellLimits {
            max_value_bytes: 16 * 1024 * 1024,
            heap_bytes: 128 * 1024 * 1024,
            ..Default::default()
        },
    );
    let (ok, output) = evaluate(
        &repl,
        bridge.clone(),
        format!("const tab=await cua.getTab({{url:{}}});", json!(page)),
    )
    .await;
    assert!(ok, "{}", texts(&output));
    let mut state = texts(&output);
    for (text, diagnostic) in [("missing", "text_not_found"), ("hello", "ambiguous_text")] {
        let name = index(&state, "textbox", "Name ");
        let (ok, output) = evaluate(&repl, bridge.clone(), format!(
            "try {{ await tab.selectText({name}, {text:?}); }} catch (e) {{ nodeRepl.write(String(e)); }}"
        )).await;
        assert!(
            ok && texts(&output).contains(diagnostic),
            "{}",
            texts(&output)
        );
        let (ok, output) = evaluate(
            &repl,
            bridge.clone(),
            "await tab.getAXState({disableDiffing:true});".into(),
        )
        .await;
        assert!(ok);
        state = texts(&output);
    }
    let name = index(&state, "textbox", "Name ");
    let message = index(&state, "textbox", "Message ");
    let color = index(&state, "combobox", "Color");
    let button = index(&state, "button", "Submit fixture");
    let (ok,output)=evaluate(&repl,bridge.clone(),format!("await tab.selectText({name}, 'hello', {{prefix:'hello '}}); await tab.typeText(null, 'world'); await tab.setValue({message}, '你好'); await tab.setValue({color}, 'blue-id'); await tab.click({button}); await tab.getAXState({{disableDiffing:true}});")).await;
    assert!(ok, "{}", texts(&output));
    assert!(
        texts(&output).contains("hello world / 你好"),
        "{}",
        texts(&output)
    );
    let (ok,output)=evaluate(&repl,bridge.clone(),"const screenshot=await tab.getScreenshot(); nodeRepl.write(screenshot instanceof Uint8Array);".into()).await;
    assert!(ok);
    assert!(
        output
            .iter()
            .any(|part| matches!(part, CellOutput::Media { .. }))
    );
    assert!(texts(&output).contains("true"));
    let (ok,output)=evaluate(&repl,bridge.clone(),format!("await tab.getAXState(); try {{await tab.setValue({color}, 'missing-option'); throw Error('invalid option accepted');}} catch (error) {{if (!String(error).includes('select option value is unavailable')) throw error;}} await tab.getAXState({{disableDiffing:true}});")).await;
    assert!(ok);
    assert!(
        texts(&output).contains("value=\"Blue\""),
        "invalid selection must preserve the selected value"
    );
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        "await tab.getScreenshot({emit:false});".into(),
    )
    .await;
    assert!(ok);
    let (ok, _) = evaluate(&repl, bridge.clone(), format!("await tab.click({button});")).await;
    assert!(
        !ok,
        "screenshot-only observation must retire element indices"
    );
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        format!(
            "await tab.getAXState(); await tab.goto({});",
            json!(format!("{page}next"))
        ),
    )
    .await;
    assert!(ok);
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        format!("await tab.setValue({name}, 'stale');"),
    )
    .await;
    assert!(!ok, "old document reference was reused");
    let (ok, _) = evaluate(
        &repl,
        bridge.clone(),
        format!("await tab.getAXState(); await tab.back(); await tab.forward(); await tab.reload(); await tab.getAXState(); const browser=await cua.getBrowser({{id:'fixture'}}); const second=await cua.createBrowserTab(browser.browserId, {}); await second.close(); await tab.close();",json!(format!("{page}second"))),
    )
    .await;
    assert!(ok);
    let (ok, _) = evaluate(&repl, bridge, "await tab.getAXState();".into()).await;
    assert!(!ok, "closed tab was rebound");
    repl.close().await.unwrap();
    session.lock().await.close().await.unwrap();
    driver.lock().await.close().await.unwrap();
    chrome.kill().await.unwrap();
    let _ = chrome.wait().await;
    logging.abort();
    server.abort();
}
