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
use maka_protocol::session::workspace_context as workspace;

fn basis() -> workspace::Basis {
    workspace::Basis {
        root_id: "root".into(),
        session_id: "chat".into(),
        boundary_revision: 1,
        workspace: crate::pages::sessions::tests::item("chat").workspace,
        directory_identity: format!("sha256:{}", "a".repeat(64)).try_into().unwrap(),
    }
}
pub(super) fn page(path: &str, kind: workspace::Kind, next: bool) -> workspace::Page {
    workspace::Page {
        basis: basis(),
        directory: path
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory)
            .into(),
        filter: String::new(),
        revision: "a".repeat(64),
        entries: vec![workspace::Entry {
            path: path.into(),
            kind,
        }],
        next_cursor: next.then(|| workspace::Cursor {
            basis: basis(),
            revision: "a".repeat(64),
            after: path.into(),
        }),
    }
}
fn activate(app: &mut App, path: &str) {
    draw(app, 60, 30);
    app.input(key(KeyCode::F(6)));
    for _ in 0..20 {
        draw(app, 60, 30);
        if app.completion.popup.as_ref().unwrap().surface.focused() == Some(path) {
            app.input(key(KeyCode::Enter));
            return;
        }
        app.input(key(KeyCode::Tab));
    }
    panic!("control is not keyboard reachable: {path}");
}

#[test]
fn controls_focus_pages_nonempty_results_browses_directories_and_previews_without_insertion() {
    for locale in Locale::ALL {
        let mut app = app(locale);
        draw(&mut app, 60, 30);
        type_text(&mut app, "@");
        let query = app.completion_request().unwrap();
        app.completion_completed(
            query,
            Ok(Output::Workspace(page(
                "first.rs",
                workspace::Kind::File,
                true,
            ))),
        );
        activate(&mut app, "completion/controls/next");
        let next = app.completion_request().unwrap();
        assert!(
            matches!(&next.job, io::Job::Workspace(query) if query.cursor.as_ref().is_some_and(|cursor| cursor.after == "first.rs"))
        );
        app.completion_completed(next, Err("candidate_set_stale".into()));
        draw(&mut app, 60, 10);
        let retry = app
            .completion
            .popup
            .as_ref()
            .unwrap()
            .surface
            .rect("completion/error-actions/retry")
            .unwrap();
        assert_eq!(
            retry.height, 1,
            "Retry remains one complete row in a short window"
        );
        assert!(retry.width > 0);
        app.input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: retry.x,
            row: retry.y,
            modifiers: KeyModifiers::NONE,
        }));
        let restarted = app.completion_request().unwrap();
        assert!(
            matches!(&restarted.job, io::Job::Workspace(query) if query.cursor.is_none() && query.directory.is_empty() && query.filter.is_empty())
        );
        app.completion_completed(
            restarted,
            Ok(Output::Workspace(page(
                "first.rs",
                workspace::Kind::File,
                true,
            ))),
        );
        activate(&mut app, "completion/controls/next");
        let next = app.completion_request().unwrap();
        app.completion_completed(
            next,
            Ok(Output::Workspace(page(
                "folder",
                workspace::Kind::Directory,
                false,
            ))),
        );
        activate(&mut app, "completion/controls/previous");
        let previous = app.completion_request().unwrap();
        assert!(matches!(&previous.job, io::Job::Workspace(query) if query.cursor.is_none()));
        app.completion_completed(
            previous,
            Ok(Output::Workspace(page(
                "first.rs",
                workspace::Kind::File,
                true,
            ))),
        );
        activate(&mut app, "completion/controls/next");
        let next = app.completion_request().unwrap();
        app.completion_completed(
            next,
            Ok(Output::Workspace(page(
                "folder",
                workspace::Kind::Directory,
                false,
            ))),
        );
        activate(
            &mut app,
            "completion/candidates/workspace:Directory:folder/title/browse",
        );
        let browse = app.completion_request().unwrap();
        assert!(
            matches!(&browse.job, io::Job::Workspace(query) if query.directory == "folder" && query.cursor.is_none())
        );
        app.completion_completed(
            browse,
            Ok(Output::Workspace(page(
                "folder/inside.rs",
                workspace::Kind::File,
                false,
            ))),
        );
        activate(&mut app, "completion/controls/preview");
        let preview = app.completion_request().unwrap();
        assert!(matches!(
            preview.job,
            io::Job::Capture { accept: false, .. }
        ));
        app.completion_completed(
            preview,
            Ok(Output::Captured(payload::workspace(&"line\n".repeat(40)))),
        );
        draw(&mut app, 60, 30);
        app.input(key(KeyCode::PageDown));
        assert!(app.completion.popup.as_ref().unwrap().preview.is_some());
        assert_eq!(app.drafts["chat"].text(), "@");
        assert!(app.drafts["chat"].marks().is_empty());
        assert!(app.sending.is_empty());
        draw(&mut app, 60, 30);
        assert!(
            app.completion
                .popup
                .as_ref()
                .unwrap()
                .surface
                .rect("completion/label")
                .is_some_and(|rect| !rect.is_empty())
        );
        app.input(key(KeyCode::Enter));
        assert!(
            !app.completion_open(),
            "presented preview must remain confirmable"
        );
        assert_eq!(app.drafts["chat"].marks().len(), 1);
        assert_eq!(
            app.submission().unwrap().content.quotes.unwrap()[0].text,
            "line\n".repeat(40)
        );
    }
}

#[test]
fn a_late_preview_never_changes_another_candidates_payload_or_confirmation() {
    let mut app = app(Locale::En);
    draw(&mut app, 60, 30);
    type_text(&mut app, "@");
    let query = app.completion_request().unwrap();
    let mut candidates = page("a.rs", workspace::Kind::File, false);
    candidates.entries.push(workspace::Entry {
        path: "b.rs".into(),
        kind: workspace::Kind::File,
    });
    app.completion_completed(query, Ok(Output::Workspace(candidates)));
    draw(&mut app, 60, 30);
    let popup = app.completion.popup.as_ref().unwrap();
    let command = Command::Preview {
        generation: popup.generation,
        id: popup.selected.clone().unwrap(),
    };
    app.completion_action(command);
    let preview = app.completion_request().unwrap();
    app.input(key(KeyCode::Down));
    assert_eq!(
        app.completion.popup.as_ref().unwrap().selected.as_deref(),
        Some("workspace:File:b.rs")
    );
    app.completion_completed(
        preview,
        Ok(Output::Captured(payload::workspace("A original quote"))),
    );
    let popup = app.completion.popup.as_ref().unwrap();
    assert!(popup.preview.is_none());
    let b = popup
        .candidates
        .iter()
        .find(|candidate| candidate.id == "workspace:File:b.rs")
        .unwrap();
    assert!(matches!(&b.pick, model::Pick::Workspace(input) if input.path == "b.rs"));
    app.input(key(KeyCode::Esc));
    assert!(!app.completion_open());
    assert!(app.drafts["chat"].marks().is_empty());
    type_text(&mut app, "b");
    let query = app.completion_request().unwrap();
    let mut b = page("b.rs", workspace::Kind::File, false);
    b.filter = "b".into();
    app.completion_completed(query, Ok(Output::Workspace(b)));
    draw(&mut app, 60, 30);
    app.input(key(KeyCode::Enter));
    let capture = app.completion_request().unwrap();
    assert!(
        matches!(&capture.job, io::Job::Capture { input, accept: true } if input.path == "b.rs")
    );
    let mut binding = payload::workspace("B own quote");
    binding.label = "b.rs".into();
    app.completion_completed(capture, Ok(Output::Captured(binding)));
    let submitted = app.submission().unwrap();
    assert_eq!(submitted.content.quotes.unwrap()[0].text, "B own quote");
}

#[test]
fn keyboard_add_menu_and_pointer_added_chip_keep_the_composer_as_input_owner() {
    for locale in Locale::ALL {
        let mut app = app(locale);
        draw(&mut app, 60, 30);
        app.input(key(KeyCode::BackTab));
        app.input(key(KeyCode::Enter));
        draw(&mut app, 60, 30);
        assert!(app.chrome.composer.captures());
        for _ in 0..3 {
            app.input(key(KeyCode::Down));
        }
        draw(&mut app, 60, 30);
        app.input(key(KeyCode::Enter));
        assert!(app.completion_open());
        assert_eq!(app.focus, crate::app::Focus::Composer);
        assert!(app.completion.popup.as_ref().unwrap().explicit);
        app.input(key(KeyCode::Esc));
        let context = app.completion_context(Some(Kind::Reference)).unwrap();
        app.completion_bind(context, payload::workspace("chip quote"));
        draw(&mut app, 60, 30);
        let chip = app
            .chrome
            .composer
            .rect("composer/body/content/context-bindings")
            .unwrap();
        app.input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: chip.x,
            row: chip.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(app.completion_open());
        assert_eq!(app.focus, crate::app::Focus::Composer);
        assert!(matches!(
            app.completion.popup.as_ref().unwrap().source,
            Source::Added
        ));
        app.input(key(KeyCode::Esc));
        type_text(&mut app, "tail");
        assert!(app.drafts["chat"].text().ends_with("tail"));
    }
}
