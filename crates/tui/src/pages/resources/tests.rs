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
use crate::{
    Locale, LocalePreference,
    app::{Action, App, ConnectionState},
    i18n::I18n,
    navigation::Route,
};
use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
fn target() -> Target {
    Target {
        root: "root".into(),
        epoch: "epoch".into(),
        session: "session".into(),
        workspace: "/captured/workspace".into(),
        policy: "workspace-write".into(),
    }
}
fn opened() -> State {
    let mut state = State::default();
    state.action(Command::Open(Box::new(target())));
    state.pending = None;
    state
}
fn resource(id: &str) -> ShellSnapshot {
    serde_json::from_value(json!({"kind":"shell_run","ref":format!("maka://runtime/background-tasks/{id}"),"mode":"pty","status":"running","cwd":"/captured/workspace","cmd":"shell","startedAt":1,"updatedAt":1,"revision":1})).unwrap()
}
#[test]
fn launch_identity_is_written_before_dispatch_and_unknown_only_queries_original_scope() {
    let mut state = opened();
    state.action(Command::RunForm);
    state.command.insert("printf private-token");
    state.action(Command::Run);
    let request = state.request().unwrap();
    assert!(request.needs_checkpoint());
    let saved = state.checkpoint().unwrap();
    saved.validate("root").unwrap();
    let bytes = serde_json::to_string(&saved).unwrap();
    assert!(!bytes.contains("private-token"));
    assert!(!bytes.contains("command"));
    assert_eq!(state.mutation, Mutation::Saving);
    assert!(state.after_checkpoint(&request, &Ok(())));
    state.complete(request, Err(Failure { unknown: true }));
    assert_eq!(state.checkpoint(), Some(saved.clone()));
    assert!(!state.enabled(&Command::NewTerminal));
    state.action(Command::Check);
    let query = state.request().unwrap();
    assert!(!query.needs_checkpoint());
    assert_eq!(query.target.session, saved.session);
    state.complete(
        query,
        Ok(Output::Query(Box::new(ResourceQueryResult::Page {
            session_id: "session".into(),
            revision: format!("sha256:{}", "a".repeat(64)),
            resources: vec![],
            next_cursor: None,
        }))),
    );
    assert_eq!(state.checkpoint(), Some(saved));
    assert!(state.request().is_none());
    assert!(!state.enabled(&Command::Run));
}
#[test]
fn closing_before_written_never_dispatches_and_late_started_receipt_still_settles_identity() {
    let mut state = opened();
    state.action(Command::NewTerminal);
    let request = state.request().unwrap();
    state.action(Command::Close);
    assert!(!state.after_checkpoint(&request, &Ok(())));
    assert!(state.checkpoint().is_some());
    let mut state = opened();
    state.action(Command::NewTerminal);
    let request = state.request().unwrap();
    assert!(state.after_checkpoint(&request, &Ok(())));
    state.action(Command::Close);
    state.complete(request, Ok(Output::Started(Box::new(resource("created")))));
    assert!(state.checkpoint().is_none());
    assert!(!state.visible);
    assert!(state.request().is_none());
}
#[test]
fn inherited_resources_never_gain_controller_or_stop_authority() {
    let mut state = opened();
    let snapshot = resource("inherited");
    let reference = snapshot.resource_ref.clone();
    state.items.insert(
        reference.clone(),
        ResourceUpdate {
            session_id: "session".into(),
            ownership: Ownership::SourceOwned {
                source_session_id: "other".into(),
                owner_session_id: "other".into(),
            },
            source_turn_id: "t".into(),
            source_tool_call_id: "c".into(),
            result: snapshot,
        },
    );
    state.action(Command::Select(reference));
    assert!(!state.enabled(&Command::Stop));
    assert!(!state.enabled(&Command::ConfirmStop));
    assert!(state.terminal_target(Some("sub")).is_none());
}

fn resource_app(locale: Locale) -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(locale), locale),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    app.chrome.motion = false;
    app.apply(Action::Visit(Route::Session("session".into())));
    let item = crate::pages::sessions::tests::item("session");
    app.sessions.detail = crate::pages::sessions::Detail::Ready(Box::new(item.clone()));
    app.sessions.items = vec![item];
    app.apply(app.resources_action().unwrap());
    app.resources.pending = None;
    app
}
fn draw(app: &mut App, width: u16, height: u16) {
    Terminal::new(TestBackend::new(width, height))
        .unwrap()
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
}
fn click(app: &mut App, path: &str) {
    let area = app.layer.rect(path).expect("visible resource control");
    assert!(!area.is_empty());
    app.input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }));
}

#[test]
fn command_form_run_is_one_visible_row_and_pointer_dispatches_at_common_and_narrow_sizes() {
    for locale in Locale::ALL {
        for (width, height) in [(80, 24), (44, 22)] {
            let mut app = resource_app(locale);
            draw(&mut app, width, height);
            click(&mut app, "catalog/create/0/resources-command");
            draw(&mut app, width, height);
            assert!(
                app.resources_presented,
                "form must fit {width}×{height} {locale:?}"
            );
            assert!(app.layer.focused("command"));
            app.input(Event::Paste("printf 'row-layout-197'".into()));
            draw(&mut app, width, height);
            let run = app
                .layer
                .rect("run/resources-run")
                .expect("Run stays visible");
            assert_eq!(
                run.height, 1,
                "button label width must not become its height"
            );
            assert!(app.enabled(&Action::Resources(Command::Run)));
            draw(&mut app, 28, 8);
            assert!(!app.enabled(&Action::Resources(Command::Run)));
            draw(&mut app, width, height);
            click(&mut app, "run/resources-run");
            let request = app
                .resources
                .request()
                .expect("pointer Run schedules a launch");
            assert!(request.needs_checkpoint());
            assert!(
                matches!(&request.work, Work::Start(input) if input.command.as_deref() == Some("printf 'row-layout-197'"))
            );
            assert!(app.resources_after_checkpoint(&request, &Ok(())));
        }
    }
}

#[test]
fn resource_catalog_buttons_are_single_rows_instead_of_label_width_tall_blocks() {
    for locale in Locale::ALL {
        for (width, height) in [(80, 24), (44, 22)] {
            let mut app = resource_app(locale);
            let mut references = Vec::new();
            for id in ["a", "b", "c", "d"] {
                let mut snapshot = resource(id);
                snapshot.cmd = format!("long enough process label {id} 中文");
                snapshot.cwd = "/work".into();
                references.push(snapshot.resource_ref.clone());
                app.resources.items.insert(
                    snapshot.resource_ref.clone(),
                    ResourceUpdate {
                        session_id: "session".into(),
                        ownership: Ownership::Local,
                        source_turn_id: id.into(),
                        source_tool_call_id: id.into(),
                        result: snapshot,
                    },
                );
            }
            draw(&mut app, width, height);
            assert!(
                app.resources_presented,
                "catalog must fit {width}×{height} {locale:?}"
            );
            let first = app
                .layer
                .rect(&format!("catalog/list/items/{}/select", references[0]))
                .unwrap();
            let second = app
                .layer
                .rect(&format!("catalog/list/items/{}/select", references[1]))
                .unwrap();
            assert_eq!(first.height, 1);
            assert_eq!(second.height, 1);
            assert_eq!(second.y, first.y + 1);
            click(
                &mut app,
                &format!("catalog/list/items/{}/select", references[0]),
            );
            draw(&mut app, width, height);
            for (index, reference) in references.iter().enumerate().skip(1).take(2) {
                assert!(app.resources_presented);
                let viewport = references
                    .iter()
                    .find_map(|reference| {
                        app.layer
                            .rect(&format!("catalog/list/items/{reference}/select"))
                            .filter(|rect| !rect.is_empty())
                    })
                    .expect("selected catalog has a visible row");
                assert_eq!(viewport.height, 1);
                let wheel = |app: &mut App, kind| {
                    app.input(Event::Mouse(MouseEvent {
                        kind,
                        column: viewport.x,
                        row: viewport.y,
                        modifiers: KeyModifiers::NONE,
                    }));
                    draw(app, width, height);
                };
                // Return to the first row using only the pointer, then visit each
                // intermediate row. A fixed three-row wheel step skips both.
                for _ in &references {
                    wheel(&mut app, MouseEventKind::ScrollUp);
                }
                for _ in 0..index {
                    wheel(&mut app, MouseEventKind::ScrollDown);
                }
                click(&mut app, &format!("catalog/list/items/{reference}/select"));
                assert_eq!(app.resources.selected.as_ref(), Some(reference));
                draw(&mut app, width, height);
            }
        }
    }
}

#[test]
fn terminal_controls_remain_visible_at_common_and_narrow_sizes_in_each_capture_state() {
    use unicode_width::UnicodeWidthStr;
    for locale in Locale::ALL {
        for (width, height) in [(80, 24), (44, 22)] {
            for captured in [false, true] {
                let mut app = resource_app(locale);
                let mut snapshot = resource("terminal");
                snapshot.cwd = "/work".into();
                let reference = snapshot.resource_ref.clone();
                app.resources.items.insert(
                    reference.clone(),
                    ResourceUpdate {
                        session_id: "session".into(),
                        ownership: Ownership::Local,
                        source_turn_id: "launch".into(),
                        source_tool_call_id: "launch".into(),
                        result: snapshot,
                    },
                );
                app.resources.action(Command::Select(reference));
                app.resources.terminal.capture = captured;
                draw(&mut app, width, height);
                assert!(
                    app.resources_presented,
                    "terminal must fit {width}×{height} {locale:?} capture={captured}"
                );
                assert!(
                    app.layer
                        .slot("terminal")
                        .is_some_and(|area| !area.is_empty())
                );
                for key in [
                    if captured {
                        "resources-controls"
                    } else {
                        "resources-type"
                    },
                    "resources-reconnect",
                ] {
                    let rect = (0..2)
                        .find_map(|row| {
                            app.layer
                                .rect(&format!("process-actions/terminal-actions/{row}/{key}"))
                        })
                        .expect("whole control is visible");
                    assert_eq!(rect.height, 1);
                    assert_eq!(
                        usize::from(rect.width),
                        app.i18n.text(key).width() + 4,
                        "the whole label must fit"
                    );
                }
            }
        }
    }
}
