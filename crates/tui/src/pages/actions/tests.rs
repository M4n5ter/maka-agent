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

mod native_resources;

use super::*;
use crate::{
    Locale, LocalePreference,
    app::Focus,
    i18n::I18n,
    pages::manage::{Command as Manage, Entity, Kind},
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use unicode_width::UnicodeWidthStr;

fn app(locale: Locale) -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(locale), locale),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    app.chrome.motion = false;
    app
}
fn session(locale: Locale) -> App {
    let mut app = app(locale);
    app.apply(Action::Visit(Route::Session("chat".into())));
    let item = super::super::sessions::tests::item("chat");
    app.sessions.detail = super::super::sessions::Detail::Ready(Box::new(item.clone()));
    app.sessions.items = vec![item];
    app.input(Event::Paste("draft 中文🦀".into()));
    app
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn click(rect: Rect) -> Event {
    assert!(!rect.is_empty(), "ordinary control must be visible");
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    })
}
fn draw(app: &mut App, width: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    terminal
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
    terminal
}
fn locate(terminal: &Terminal<TestBackend>, label: &str) -> Rect {
    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let mut row = String::new();
        let mut x = 0;
        while x < buffer.area.width {
            let text = buffer[(x, y)].symbol();
            row.push_str(text);
            x += text.width().max(1) as u16;
        }
        if let Some(byte) = row.find(label) {
            return Rect::new(row[..byte].width() as u16, y, label.width() as u16, 1);
        }
    }
    panic!("missing visible action: {label}");
}

#[test]
fn new_message_add_menu_reaches_each_source_by_keyboard_and_pointer_in_every_locale() {
    for locale in Locale::ALL {
        for width in [30, 48, 80] {
            for pointer in [false, true] {
                for index in 0..3 {
                    let mut app = session(locale);
                    draw(&mut app, width);
                    if pointer {
                        let plus = app
                            .chrome
                            .composer
                            .rect("composer/body/leading/buttons/attach")
                            .unwrap();
                        app.input(click(plus));
                    } else {
                        app.input(key(KeyCode::BackTab));
                        app.input(key(KeyCode::Enter));
                    }
                    let terminal = draw(&mut app, width);
                    assert!(app.chrome.composer.captures());
                    assert!(
                        app.input(Event::Key(KeyEvent::new(
                            KeyCode::Enter,
                            KeyModifiers::CONTROL
                        )))
                        .1
                        .is_none(),
                        "an open add menu cannot send through to the composer"
                    );
                    if pointer {
                        let label = app
                            .i18n
                            .text(["attachments-add", "references-title", "skills-title"][index]);
                        app.input(click(locate(&terminal, &label)));
                    } else {
                        for _ in 0..index {
                            app.input(key(KeyCode::Down));
                        }
                        draw(&mut app, width);
                        app.input(key(KeyCode::Enter));
                    }
                    match index {
                        0 => assert!(app.attachments.dialog.is_some()),
                        1 => assert!(
                            app.management
                                .dialog
                                .as_ref()
                                .is_some_and(|dialog| dialog.kind == Kind::Reference)
                        ),
                        _ => assert!(app.skills.dialog.is_some()),
                    }
                    assert!(app.palette.is_none());
                    assert_eq!(app.drafts["chat"].text(), "draft 中文🦀");
                }
            }
        }
    }
}

#[test]
fn session_actions_and_compact_model_controls_open_existing_dialogs_without_palette() {
    let mut app = session(Locale::En);
    draw(&mut app, 30);
    let options = app
        .chrome
        .composer
        .rect("composer/metadata/options")
        .expect("narrow model must stay reachable");
    app.input(click(options));
    let terminal = draw(&mut app, 30);
    app.input(click(locate(
        &terminal,
        &app.i18n.text("session-model-change"),
    )));
    assert!(
        app.management
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.kind == Kind::Model)
    );
    app.apply(Action::Manage(Manage::Close));
    draw(&mut app, 80);
    let menu = app
        .chrome
        .header
        .rect("header/right/session-actions")
        .unwrap();
    app.input(click(menu));
    let terminal = draw(&mut app, 80);
    app.input(click(locate(&terminal, &app.i18n.text("session-rename"))));
    assert!(
        app.management
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.kind == Kind::Rename
                && matches!(&dialog.target.entity, Entity::Session { id, .. } if id == "chat"))
    );
    assert!(app.palette.is_none());
    assert_eq!(app.drafts["chat"].text(), "draft 中文🦀");
}

#[test]
fn normal_header_menu_marks_the_message_exposed_by_the_current_draw() {
    let mut app = session(Locale::En);
    app.chat.select(&Route::Session("chat".into()));
    app.chat.snapshot = Some(maka_protocol::subscription::decode_session_observation_snapshot(
        &serde_json::json!({
            "schemaVersion":5,
            "session":{"sessionId":"chat","metadataRevision":1,"status":"active","createdAt":0,"isArchived":false},
            "projectionRevision":1,"rootTurn":null,"goal":null,
            "queue":{"hostEpoch":"epoch","queueRevision":0,"steering":[],"followup":[]},
            "interactions":{"pending":[]}
        }),
    ).unwrap());
    app.chat.fixture_rows(std::collections::BTreeMap::from([(
        1,
        serde_json::json!({"type":"user","id":"message","turnId":"turn","text":"A durable message"}),
    )]));
    draw(&mut app, 80);
    let menu = app
        .chrome
        .header
        .rect("header/right/session-actions")
        .unwrap();
    app.input(click(menu));
    let terminal = draw(&mut app, 80);
    app.input(click(locate(
        &terminal,
        &app.i18n.text("controls-mark-read"),
    )));
    assert!(app.session_controls.visible);
    assert!(app.palette.is_none());
    let terminal = draw(&mut app, 80);
    app.input(click(locate(&terminal, &app.i18n.text("plugins-confirm"))));
    let saved = serde_json::to_value(app.session_controls.checkpoint().unwrap()).unwrap();
    assert_eq!(saved["mutation"]["kind"], "read");
    assert_eq!(saved["mutation"]["input"]["sessionId"], "chat");
    assert_eq!(
        saved["mutation"]["input"]["readThroughMessageId"],
        "message"
    );
}

#[test]
fn connection_row_menu_keeps_its_own_target_while_another_row_is_selected() {
    let mut app = app(Locale::En);
    app.apply(Action::Visit(Route::Connections));
    app.providers = crate::providers::fixtures::catalog();
    app.connections.refresh();
    app.connections.query().unwrap();
    let provider = crate::providers::fixtures::entry("openai-compatible", false).identity;
    app.connections.complete(Ok(serde_json::json!({"kind":"page","revision":1,"connectionCount":2,"nextCursor":null,"defaultTarget":null,
        "items": [
            {"kind":"connection","connectionIndex":0,"connectionId":"a","revision":1,"slug":"a","name":"First","provider":provider,"configuration":{},"requestBodyOverlay":{},"enabled":true,"enabledModelIdCount":0},
            {"kind":"connection","connectionIndex":1,"connectionId":"b","revision":1,"slug":"b","name":"Second","provider":provider,"configuration":{},"requestBodyOverlay":{},"enabled":true,"enabledModelIdCount":0}
        ]})));
    draw(&mut app, 80);
    assert_eq!(app.connections.selected.as_deref(), Some("a"));
    let menu = app
        .connections
        .surface
        .rect("connections/rows/b/actions")
        .unwrap();
    app.input(click(menu));
    let terminal = draw(&mut app, 80);
    app.input(click(locate(
        &terminal,
        &app.i18n.text("connection-rename"),
    )));
    assert!(app.management.dialog.as_ref().is_some_and(
        |dialog| matches!(&dialog.target.entity, Entity::Connection(row) if row.id == "b")
    ));
    assert!(app.palette.is_none());
}

#[test]
fn home_projects_and_project_keyboard_menu_use_normal_page_controls() {
    let mut app = app(Locale::En);
    let terminal = draw(&mut app, 80);
    app.input(click(locate(&terminal, &app.i18n.text("route-projects"))));
    assert_eq!(app.navigation.current(), Route::Projects);
    app.projects.loaded = true;
    app.projects.items = vec![super::super::projects::Item {
        id: "p".into(),
        name: "Project".into(),
        archived: false,
        available: true,
    }];
    app.projects.selected = Some("p".into());
    app.focus = Focus::List;
    draw(&mut app, 48);
    app.projects.surface.focus("projects/rows/p/summary".into());
    app.input(key(KeyCode::Right));
    app.input(key(KeyCode::Enter));
    let terminal = draw(&mut app, 48);
    assert!(app.projects.surface.captures());
    app.input(click(locate(&terminal, &app.i18n.text("project-rename"))));
    assert!(
        app.management
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.kind == Kind::Rename
                && matches!(&dialog.target.entity, Entity::Project { id } if id == "p"))
    );
    assert!(app.palette.is_none());
}

#[test]
fn reader_rebase_between_menu_paint_and_click_cannot_copy_another_message() {
    use crate::ui::transcript::{MessageKey, Part, selection::CopyMode};
    let mut app = session(Locale::En);
    let rows = std::collections::BTreeMap::from([
        (
            1,
            serde_json::json!({"turnId":"turn","id":"a","type":"user","text":"Original"}),
        ),
        (
            2,
            serde_json::json!({"turnId":"turn","id":"b","type":"user","text":"Another"}),
        ),
    ]);
    app.chat.view.sync(&rows, &[], 0, &app.i18n, false);
    app.chat
        .view
        .select(MessageKey::new("turn", "a", Part::Text));
    draw(&mut app, 80);
    app.input(click(
        app.chrome
            .header
            .rect("header/right/session-actions")
            .unwrap(),
    ));
    let terminal = draw(&mut app, 80);
    let copy = locate(&terminal, &app.i18n.text(CopyMode::Message.label()));
    app.chat
        .view
        .select(MessageKey::new("turn", "b", Part::Text));
    assert!(app.input(click(copy)).1.is_none());
    assert!(!app.chrome.header.captures());
    assert!(app.palette.is_none());

    for mode in [CopyMode::Message, CopyMode::Source] {
        let mut app = session(Locale::En);
        app.chat.select(&Route::Session("chat".into()));
        app.chat.snapshot = Some(maka_protocol::subscription::decode_session_observation_snapshot(
            &serde_json::json!({
                "schemaVersion":5,
                "session":{"sessionId":"chat","metadataRevision":1,"status":"active","createdAt":0,"isArchived":false},
                "projectionRevision":1,"rootTurn":null,"goal":null,
                "queue":{"hostEpoch":"epoch","queueRevision":0,"steering":[],"followup":[]},
                "interactions":{"pending":[]}
            }),
        ).unwrap());
        app.chat.fixture_rows(rows.clone());
        app.chat
            .view
            .search_command(crate::ui::transcript::search::Command::Open);
        draw(&mut app, 80);
        assert!(app.chat.view.selected().is_none());
        assert_eq!(app.chat.view.selection().unwrap().message(), "a");
        app.input(click(
            app.chrome
                .header
                .rect("header/right/session-actions")
                .unwrap(),
        ));
        draw(&mut app, 80);
        app.input(key(KeyCode::End));
        draw(&mut app, 80);

        let mut appended = rows.clone();
        for sequence in 3..15 {
            appended.insert(sequence, serde_json::json!({"turnId":"turn","id":format!("background-{sequence}"),"type":"assistant","text":"New background content"}));
        }
        app.chat.fixture_rows(appended.clone());
        let terminal = draw(&mut app, 80);
        assert_ne!(app.chat.view.selection().unwrap().message(), "a");
        assert!(
            app.chrome.header.captures(),
            "background reading changes leave session operations available"
        );
        let action = app
            .input(click(locate(&terminal, &app.i18n.text(mode.label()))))
            .1
            .unwrap();
        let Action::CopyMessage {
            target,
            mode: copied_mode,
        } = action
        else {
            panic!("bound message copy");
        };
        assert_eq!(copied_mode, mode);
        assert_eq!(target.text(&app, mode).unwrap(), "Original");
        assert!(
            app.chat.view.search.is_some(),
            "copy keeps its search reader alive"
        );

        appended.remove(&1);
        app.chat.fixture_rows(appended);
        draw(&mut app, 80);
        assert!(
            target.text(&app, mode).is_err(),
            "a retired source cannot fall back to another visible message"
        );
        app.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "replacement".into(),
        };
        assert!(!target.current(&app));
    }
}
