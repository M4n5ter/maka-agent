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

use super::controls::page;
use super::*;
use maka_protocol::session::workspace_context as workspace;

fn multiline() -> App {
    let mut app = app(Locale::En);
    draw(&mut app, 40, 10);
    app.input(Event::Paste("first line\nsecond line\nthird line".into()));
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Home,
        KeyModifiers::CONTROL,
    )));
    // A real two-row composer puts its first-line caret farther from the
    // bottom edge. At the supported minimum height, the popup border fits
    // while its explicit query and controls consume all candidate rows.
    draw(&mut app, 40, 10);
    assert_eq!(app.drafts["chat"].save().cursor, 0);
    assert!(app.drafts["chat"].cursor_position().is_some());
    app
}

#[test]
fn a_short_explicit_picker_never_accepts_a_candidate_hidden_by_its_controls() {
    let mut app = multiline();
    app.completion_action(Command::Open(Kind::Reference));
    let query = app.completion_request().unwrap();
    app.completion_completed(
        query,
        Ok(Output::Workspace(page(
            "hidden.rs",
            workspace::Kind::File,
            false,
        ))),
    );
    for height in [10, 12] {
        let mut terminal = Terminal::new(TestBackend::new(40, height)).unwrap();
        terminal
            .draw(|frame| crate::view::draw(frame, &mut app))
            .unwrap();
        let popup = app.completion.popup.as_ref().unwrap();
        let area = popup.area.expect("the popup border is actually presented");
        assert!(
            popup
                .surface
                .rect("completion/candidates/workspace:File:hidden.rs/title/name")
                .is_none_or(|rect| rect.is_empty()),
            "a title/detail pair that cannot fit must not overlap the footer"
        );
        let select = popup.surface.rect("completion/controls/select").unwrap();
        let footer = (area.x + 1..area.right() - 1)
            .flat_map(|x| terminal.backend().buffer()[(x, select.y)].symbol().chars())
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        assert_eq!(
            footer, "PreviewSelect",
            "no candidate text fragments may remain under controls"
        );
        for code in [KeyCode::Enter, KeyCode::Tab] {
            app.input(key(code));
            assert!(
                app.completion_request().is_none(),
                "unseen rows cannot start capture"
            );
            assert!(app.drafts["chat"].marks().is_empty());
            assert!(app.sending.is_empty());
        }
    }
    draw(&mut app, 40, 30);
    app.input(key(KeyCode::Enter));
    assert!(matches!(
        app.completion_request().unwrap().job,
        io::Job::Capture { accept: true, .. }
    ));
}

#[test]
fn a_wrapped_added_footer_cannot_change_an_unpresented_binding() {
    use crate::pages::completion::{Origin, Payload};
    use maka_runtime::input::{QuoteRef, SelectionSource};
    let mut app = multiline();
    let context = app.completion_context(Some(Kind::Reference)).unwrap();
    app.completion_bind(
        context,
        Binding {
            label: "Record seven".into(),
            origin: Origin::Plugin {
                title: "Records".into(),
                package: "records".into(),
            },
            inline: None,
            payload: Payload::Selection {
                provider: "records".into(),
                selector: "7".into(),
                source: Some(SelectionSource {
                    provider: "records".into(),
                    package_id: "records".into(),
                    entry_id: "records".into(),
                    activation: "original".into(),
                    registration: uuid::Uuid::new_v4(),
                    session_id: "chat".into(),
                }),
                quote: Some(QuoteRef {
                    text: "kept original quote".into(),
                    label: None,
                    source_turn_id: None,
                    source: None,
                }),
            },
        },
    );
    app.completion_action(Command::Added);
    draw(&mut app, 40, 12);
    let draft = app.completion_draft().unwrap();
    let marks = app.drafts["chat"].marks().to_vec();
    let before = app.completion_bindings(&draft).unwrap().clone();
    let popup = app.completion.popup.as_ref().unwrap();
    let id = popup.selected.as_ref().unwrap();
    assert!(
        popup
            .surface
            .rect(&format!("completion/candidates/{id}/title/name"))
            .is_none_or(|rect| rect.is_empty())
    );
    let remove = popup.surface.rect("completion/controls/remove").unwrap();
    assert_eq!(
        remove.height, 2,
        "this footer requires both rows at its actual width"
    );
    for control in ["reselect", "excerpt", "remove"] {
        let rect = app
            .completion
            .popup
            .as_ref()
            .unwrap()
            .surface
            .rect(&format!("completion/controls/{control}"))
            .unwrap();
        assert!(!rect.is_empty());
        app.input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.drafts["chat"].marks(), marks);
        assert_eq!(app.completion_bindings(&draft).unwrap(), &before);
        assert!(app.completion_request().is_none());
    }
    draw(&mut app, 40, 30);
    let remove = app
        .completion
        .popup
        .as_ref()
        .unwrap()
        .surface
        .rect("completion/controls/remove")
        .unwrap();
    app.input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: remove.x,
        row: remove.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(
        app.drafts["chat"].marks().is_empty(),
        "a presented binding can still be removed"
    );
}
