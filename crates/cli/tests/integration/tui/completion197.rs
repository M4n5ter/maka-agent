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
use maka_client::Client;
use maka_protocol::{
    session::{decode_session_create_input, sources},
    subscription::{SubscriptionOpenInput, TranscriptPolicy},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

mod resources;

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

async fn model(
    root: &std::path::Path,
    workspace: &std::path::Path,
    session: &str,
    title: &str,
    answer: &'static str,
    calls: Arc<AtomicUsize>,
) -> (Client, tokio::task::JoinHandle<Value>) {
    use tokio::{io::AsyncWriteExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = support::model_client(
        root,
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    )
    .await;
    client
        .create_session(
            decode_session_create_input(&json!({"sessionId":session,"name":title,
        "workspace":{"kind":"host_path","path":workspace},"sandboxMode":"read-only",
        "modelTarget":{"kind":"default"}}))
            .unwrap(),
        )
        .await
        .unwrap();
    let provider = tokio::spawn(async move {
        let (mut stream, body) = model_request(&listener).await;
        calls.fetch_add(1, Ordering::SeqCst);
        let frame = json!({"id":"completion197","object":"chat.completion.chunk","model":"fixture-model",
            "choices":[{"index":0,"delta":{"content":answer},"finish_reason":"stop"}]});
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {frame}\n\ndata: [DONE]\n\n").as_bytes()).await.unwrap();
        body
    });
    (client, provider)
}

fn open_session(tui: &mut Pty, title: &str) {
    tui.wait_for(title);
    tui.click_text(title);
    tui.wait_for("No messages yet.");
    tui.wait_for("Message…");
    tui.click_page_text("Message…");
}

fn typed(tui: &mut Pty, value: &str) {
    // Raw terminal keys, deliberately distinct from bracketed paste.
    for character in value.chars() {
        tui.send(character.encode_utf8(&mut [0; 4]).as_bytes());
    }
}

fn heading(screen: &str, label: &str) -> bool {
    screen.contains(&format!("{label} ▾")) || screen.contains(&format!("{label} v"))
}

fn composer_has(screen: &str, text: &str) -> bool {
    // These fixed-size fixtures use one-line drafts; candidates and chips sit above them.
    let echo = format!("+  {text} ");
    screen
        .lines()
        .rev()
        .take(3)
        .any(|line| line.contains(&echo))
}

fn candidates_settled(screen: &str) -> bool {
    // Existing candidates remain visible while a newer query has a footer spinner.
    !screen.contains("Loading…") && !screen.contains("Select …")
}

fn category(tui: &mut Pty, current: &str, selected: &str) {
    tui.wait_until(|screen| heading(screen, current) && candidates_settled(screen));
    let unicode = format!("{current} ▾");
    let ascii = format!("{current} v");
    let snapshot = tui.screen.snapshot().unwrap().screen;
    tui.click_text(if snapshot.contains(&unicode) {
        &unicode
    } else {
        &ascii
    });
    tui.wait_for("Added context");
    tui.click_page_text(selected);
    tui.wait_until(|screen| heading(screen, selected));
}

fn no_palette(tui: &Pty) {
    assert!(
        !tui.screen
            .snapshot()
            .unwrap()
            .screen
            .contains("Search commands…")
    );
}

async fn source(client: &Client, session: &str) -> sources::Source {
    let opened = client
        .open_subscription(SubscriptionOpenInput {
            session_id: session.into(),
            transcript: TranscriptPolicy::None,
        })
        .await
        .unwrap();
    let turn_id = opened.snapshot.root_turn.unwrap().turn_id;
    client
        .close_subscription(&opened.subscription_id)
        .await
        .unwrap();
    let sources = client
        .session_turn_sources(sources::Input {
            session_id: session.into(),
            turn_id,
        })
        .await
        .unwrap();
    assert_eq!(sources.messages.len(), 1);
    sources.messages.into_iter().next().unwrap()
}

#[test]
fn native_composer_slash_and_workspace_completion_restore_real_context_before_send() {
    const SESSION: &str = "completion-native";
    const PROOF: &str = "NATIVE_CONTEXT_197 中文🦀";
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let path = workspace.join("notes-中文.txt");
    std::fs::write(&path, format!("{PROOF}\nSecond canonical line.")).unwrap();
    let mut host = start(directory.path().join("root"));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (client, provider) = runtime.block_on(model(
        &host.root,
        &workspace,
        SESSION,
        "Completion native",
        "Native captured context accepted.",
        calls.clone(),
    ));
    let args = ["--root", host.root.to_str().unwrap()];
    let mut tui = Pty::spawn(&args);
    open_session(&mut tui, "Completion native");
    tui.send(b"\x1b[200~/help\x1b[201~");
    tui.wait_until(|screen| composer_has(screen, "/help"));
    assert!(!heading(&tui.screen.snapshot().unwrap().screen, "Commands"));
    tui.send(b"\x01\x7f");
    tui.wait_for("Message…");
    for keyboard in [true, false] {
        let phase = if keyboard { "keyboard" } else { "pointer" };
        eprintln!("completion197 /help {phase}: typing");
        typed(&mut tui, "/help");
        tui.wait_until(|screen| {
            composer_has(screen, "/help")
                && heading(screen, "Commands")
                && screen.contains("/help  Keyboard and mouse help")
                && candidates_settled(screen)
        });
        no_palette(&tui);
        if keyboard {
            tui.send(b"\r");
        } else {
            tui.click_text("Keyboard and mouse help");
        }
        tui.wait_for("Move focus between controls");
        eprintln!("completion197 /help {phase}: help opened");
        assert!(!heading(&tui.screen.snapshot().unwrap().screen, "Commands"));
        no_palette(&tui);
        // The shortcut body also says "Close tab"; the final match is the footer button.
        tui.click_last_text("Close");
        tui.wait_until(|screen| {
            !screen.contains("Move focus between controls") && screen.contains("Message…")
        });
        eprintln!("completion197 /help {phase}: help closed");
        tui.click_page_text("Message…");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "slash discovery/help cannot submit a message"
    );
    typed(&mut tui, "@notes");
    tui.wait_until(|screen| {
        composer_has(screen, "@notes")
            && heading(screen, "Workspace")
            && screen.contains("notes-中文.txt")
            && screen.contains("Preview")
            && candidates_settled(screen)
    });
    tui.send(b"\t");
    tui.wait_until(|screen| {
        composer_has(screen, "@notes-中文.txt") && !heading(screen, "Workspace")
    });
    tui.send(b"\x1a");
    tui.wait_until(|screen| composer_has(screen, "@notes") && !screen.contains("@notes-中文.txt"));
    tui.send(b"\x19");
    tui.wait_until(|screen| {
        composer_has(screen, "@notes-中文.txt") && !heading(screen, "Workspace")
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "capture and Undo/Redo cannot execute input"
    );
    no_palette(&tui);
    tui.close_terminal();
    tui.finish();
    let checkpoint = directory
        .path()
        .join("tui-state")
        .join(&client.identity.root_id)
        .join("default/state.json");
    let saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    assert_eq!(
        saved["drafts"][SESSION]["marks"].as_array().unwrap().len(),
        1
    );
    assert!(saved["completion"].to_string().contains(PROOF));
    assert!(saved["unresolved"].as_array().unwrap().is_empty());
    // Recovery must use the already selected immutable excerpt, not reread the path.
    std::fs::write(&path, "CHANGED_AFTER_CAPTURE_197").unwrap();
    let mut reopened = Pty::spawn(&args);
    reopened.wait_until(|screen| composer_has(screen, "@notes-中文.txt"));
    reopened.wait_for("fixture-model");
    reopened.click_last_text("@notes-中文.txt");
    reopened.send(b"\r");
    reopened.wait_for("Native captured context accepted.");
    no_palette(&reopened);
    let body = runtime.block_on(provider).unwrap();
    assert!(body.to_string().contains(PROOF));
    assert!(!body.to_string().contains("CHANGED_AFTER_CAPTURE_197"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let original = runtime.block_on(source(&client, SESSION));
    assert!(original.content.text.contains("@notes-中文.txt"));
    let quotes = original.content.quotes.unwrap();
    assert_eq!(quotes.len(), 1);
    assert!(quotes[0].text.contains(PROOF));
    assert!(!quotes[0].text.contains("CHANGED_AFTER_CAPTURE_197"));
    assert!(original.input_selection_sources.is_empty());
    reopened.close_terminal();
    reopened.finish();
    client.disconnect();
    host.retire_registered();
    assert!(host.wait_for_exit().success());
}
