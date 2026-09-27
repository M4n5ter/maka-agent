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
use crossterm::event::{Event as Input, KeyCode, KeyEvent, KeyModifiers};
fn target() -> Target {
    Target {
        root: "root".into(),
        epoch: "epoch".into(),
        session: "session".into(),
        subscription: "sub".into(),
        resource_ref: "maka://runtime/background-tasks/r".into(),
        generation: 1,
    }
}
fn canonical(view: &mut View, text: &str) {
    view.canonical(Some(&ShellOutput::Pty {
        screen: text.into(),
        scrollback: "earlier line\n".into(),
        last_alternate_screen: None,
        cols: 80,
        rows: 24,
        cursor: TerminalCursor {
            x: 0,
            y: 1,
            visible: true,
        },
        alternate_screen: false,
        truncated: true,
        redacted: false,
    }));
}
fn acquired(view: &mut View, target: Target, buffer: &str, sequence: u64) {
    let token = view.token.unwrap_or_else(Uuid::new_v4);
    view.event(Event {
        target: target.clone(),
        token,
        update: Update::Acquired(ControllerAcquireResult {
            controller_id: token.to_string(),
            next_sequence: 1,
            pty: maka_protocol::resource::PtySnapshot {
                session_id: target.session,
                resource_ref: target.resource_ref,
                sequence,
                buffer: buffer.into(),
                size: TerminalSize::new(80, 24).unwrap(),
            },
        }),
    });
}
#[test]
fn native_input_stays_typed_until_the_host_cut_and_capture_has_an_escape() {
    let mut view = View::default();
    let target = target();
    view.bind(Some(target.clone()));
    canonical(&mut view, "中文 🚀");
    acquired(&mut view, target, "ignored raw tail", 3);
    view.capture = true;
    assert!(view.input(&Input::Paste("echo one\necho two\x1b[201~".into())));
    let Some((Control::Write(PtyControl::Actions { actions }), _)) = view.queue.pop_front() else {
        panic!("missing input")
    };
    assert_eq!(
        actions,
        vec![InputAction::Paste("echo one\necho two[201~".into())]
    );
    assert!(view.screen.as_ref().unwrap().screen.contains("中文 🚀"));
    assert!(view.input(&Input::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    ))));
    let Some((Control::Write(PtyControl::Actions { actions }), _)) = view.queue.pop_front() else {
        panic!("missing interrupt")
    };
    assert_eq!(
        serde_json::to_value(actions).unwrap(),
        serde_json::json!([{"type":"key","key":"c","modifiers":["ctrl"]}])
    );
    for key in [']', '5'] {
        view.capture = true;
        view.syncing = true;
        assert!(view.input(&Input::Key(KeyEvent::new(
            KeyCode::Char(key),
            KeyModifiers::CONTROL
        ))));
        assert!(!view.capture);
        assert!(view.queue.is_empty());
    }
}
#[test]
fn long_history_and_control_string_tails_never_replace_canonical_screen() {
    let mut view = View::default();
    let target = target();
    view.bind(Some(target.clone()));
    canonical(
        &mut view,
        "home marker from before the bounded tail\nvisible 中",
    );
    let tail = format!("{}\x07\x1b[2J", "formerly hidden OSC payload".repeat(2000));
    acquired(&mut view, target.clone(), &tail, 2);
    assert_eq!(
        view.screen.as_ref().unwrap().screen,
        "home marker from before the bounded tail\nvisible 中"
    );
    assert!(!view.syncing);
    assert!(view.queue.is_empty());
    assert!(!view.screen.as_ref().unwrap().screen.contains("payload"));
    canonical(&mut view, "fresh Host cut");
    assert_eq!(view.screen.as_ref().unwrap().screen, "fresh Host cut");
    let mut other = target;
    other.generation += 1;
    view.bind(Some(other));
    assert!(view.screen.is_none());
}
#[test]
fn canonical_output_does_not_grant_capture_and_resize_preserves_held_input() {
    let mut view = View::default();
    let target = target();
    view.bind(Some(target.clone()));
    canonical(&mut view, "output arrives before Acquire");
    assert!(
        !view.ready(),
        "a readable screen is not controller ownership"
    );
    acquired(&mut view, target, "unused tail", 1);
    assert!(view.ready());
    view.capture = true;
    view.syncing = true; // A resize was sent after the enabled control was drawn.
    assert!(
        view.ready(),
        "resize does not revoke a held capture capability"
    );
    assert!(view.input(&Input::Key(KeyEvent::new(
        KeyCode::Char('x'),
        KeyModifiers::NONE
    ))));
    assert!(
        matches!(view.queue.front(), Some((Control::Write(PtyControl::Actions { actions }), _))
        if actions == &[InputAction::Text("x".into())])
    );
    assert!(view.capture);
    view.resize(60, 20);
    assert!(view.resize.is_some());
    view.resize(80, 24);
    assert!(
        view.resize.is_none(),
        "coalescing drops a resize already reflected by the current cut"
    );
}
mod runner;
