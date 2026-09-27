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
use crate::{Locale, LocalePreference, i18n::I18n};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};

mod controls;
mod payload;
mod performance;
mod visibility;

fn app(locale: Locale) -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(locale), locale),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    app.apply(Action::Visit(Route::Session("chat".into())));
    let item = crate::pages::sessions::tests::item("chat");
    app.sessions.detail = crate::pages::sessions::Detail::Ready(Box::new(item.clone()));
    app.sessions.items = vec![item];
    app
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.input(key(KeyCode::Char(ch)));
    }
}
fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
}

#[test]
fn inline_commands_use_keyboard_and_pointer_without_send_through_in_every_locale() {
    for locale in Locale::ALL {
        for width in [30, 80] {
            for pointer in [false, true] {
                let mut app = app(locale);
                draw(&mut app, width, 30);
                type_text(&mut app, "/help");
                assert!(app.completion_open());
                // A reply cannot make a candidate actionable until a frame presents it.
                app.input(key(KeyCode::Enter));
                assert!(!app.help);
                assert_eq!(app.drafts["chat"].text(), "/help");
                draw(&mut app, width, 30);
                let popup = app.completion.popup.as_ref().unwrap();
                assert_eq!(popup.candidates.len(), 1);
                assert_eq!(
                    popup.area.unwrap().height,
                    7,
                    "one candidate needs header, hint, title/detail, controls and borders"
                );
                if pointer {
                    let rect = app
                        .completion
                        .popup
                        .as_ref()
                        .unwrap()
                        .surface
                        .rect("completion/candidates/native:help/title/name")
                        .unwrap();
                    app.input(Event::Mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: rect.x,
                        row: rect.y,
                        modifiers: KeyModifiers::NONE,
                    }));
                } else {
                    app.input(key(KeyCode::Tab));
                }
                assert!(app.help);
                assert!(app.drafts["chat"].text().is_empty());
                assert!(!app.completion_open());
                assert!(app.sending.is_empty());
            }
        }
    }
}

#[test]
fn paste_stale_tokens_and_cancelled_queries_never_bind_or_execute() {
    let mut app = app(Locale::En);
    draw(&mut app, 80, 30);
    app.input(Event::Paste("/help".into()));
    assert!(!app.completion_open());
    assert!(!app.help);
    *app.drafts.get_mut("chat").unwrap() = Editor::default();
    type_text(&mut app, "@");
    let first = app.completion_request().unwrap();
    type_text(&mut app, "a");
    assert!(first.cancel.cancelled());
    assert!(
        app.completion_request().is_none(),
        "new query waits for resource cleanup"
    );
    let binding = payload::workspace("Captured");
    app.completion_completed(first, Ok(Output::Captured(binding)));
    assert!(app.drafts["chat"].marks().is_empty());
    let second = app.completion_request().unwrap();
    app.input(key(KeyCode::Esc));
    assert!(second.cancel.cancelled());
    app.completion_completed(second, Err("cancelled".into()));
    assert!(!app.completion_open());
    assert_eq!(app.drafts["chat"].text(), "@a");
    type_text(&mut app, "b");
    draw(&mut app, 10, 4);
    app.input(key(KeyCode::Enter));
    assert_eq!(app.drafts["chat"].text(), "@ab");
    assert!(app.sending.is_empty());
}

#[test]
fn workspace_candidate_capture_is_a_single_input_transaction_for_keys_and_pointer() {
    use maka_protocol::session::workspace_context as workspace;
    for locale in Locale::ALL {
        for pointer in [false, true] {
            let mut app = app(locale);
            draw(&mut app, 48, 30);
            type_text(&mut app, "@中");
            let query = app.completion_request().unwrap();
            let io::Job::Workspace(input) = &query.job else {
                panic!("expected workspace query")
            };
            let page = workspace::Page {
                basis: workspace::Basis {
                    root_id: "root".into(),
                    session_id: "chat".into(),
                    boundary_revision: 1,
                    workspace: crate::pages::sessions::tests::item("chat").workspace,
                    directory_identity: format!("sha256:{}", "a".repeat(64)).try_into().unwrap(),
                },
                directory: input.directory.clone(),
                filter: input.filter.clone(),
                revision: "a".repeat(64),
                entries: vec![workspace::Entry {
                    path: "中文.rs".into(),
                    kind: workspace::Kind::File,
                }],
                next_cursor: None,
            };
            app.completion_completed(query, Ok(Output::Workspace(page)));
            draw(&mut app, 48, 30);
            if pointer {
                let popup = app.completion.popup.as_ref().unwrap();
                let id = popup.selected.as_ref().unwrap();
                let rect = popup
                    .surface
                    .rect(&format!("completion/candidates/{id}/title/name"))
                    .unwrap();
                app.input(Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x,
                    row: rect.y,
                    modifiers: KeyModifiers::NONE,
                }));
            } else {
                app.input(key(KeyCode::Enter));
            }
            let capture = app.completion_request().unwrap();
            assert!(matches!(capture.job, io::Job::Capture { accept: true, .. }));
            app.input(key(KeyCode::Enter));
            assert!(app.completion_request().is_none());
            app.completion_completed(
                capture,
                Ok(Output::Captured(payload::workspace("完整正文 🦀"))),
            );
            assert!(!app.completion_open());
            assert_eq!(app.drafts["chat"].text(), "@中文.rs ");
            assert_eq!(app.drafts["chat"].marks().len(), 1);
            assert!(app.sending.is_empty());
        }
    }
}

#[test]
fn keyboard_category_menu_reaches_sessions_and_plugins_in_each_locale() {
    for locale in Locale::ALL {
        let mut app = app(locale);
        draw(&mut app, 48, 30);
        type_text(&mut app, "@");
        for expected in [Category::Sessions, Category::Plugins] {
            draw(&mut app, 48, 30);
            app.input(key(KeyCode::BackTab));
            assert!(app.completion.popup.as_ref().unwrap().surface.captures());
            draw(&mut app, 48, 30);
            app.input(key(KeyCode::Down));
            draw(&mut app, 48, 30);
            app.input(key(KeyCode::Enter));
            assert_eq!(
                app.completion.popup.as_ref().unwrap().source.category(),
                expected
            );
            assert!(!app.completion.popup.as_ref().unwrap().surface.captures());
            assert_eq!(app.drafts["chat"].text(), "@");
            assert!(app.sending.is_empty());
        }
    }
}
