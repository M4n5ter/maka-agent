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

use crate::{
    app::Action,
    pages::{
        completion::{self, Binding, Origin, Payload},
        revision::{
            self, Command, Output,
            saved::Stage,
            tests::{frame, sources},
        },
    },
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use maka_protocol::plugin::{InputResourceProjection, Page};
use maka_runtime::input::SelectionSource;

#[test]
fn copied_revision_pauses_and_keeps_old_source_until_explicit_new_scope_confirmation() {
    let (mut app, basis) = crate::pages::branch::tests::fixture();
    let old = SelectionSource {
        provider: "records".into(),
        package_id: "records".into(),
        entry_id: "records".into(),
        activation: "old".into(),
        registration: uuid::Uuid::new_v4(),
        session_id: "source".into(),
    };
    let mut original = sources("source");
    original.messages[0]
        .input_selections
        .insert("records".into(), vec!["7".into()]);
    original.messages[0]
        .input_selection_sources
        .push(old.clone());
    app.apply(Action::Revision(Command::Open(basis)));
    let load = app.revision_request().unwrap();
    app.revision_completed(load, Ok(Output::Sources(original.clone())));
    frame(&mut app, 80, 30);
    app.apply(Action::Revision(Command::Send));
    let copy = app.revision_request().unwrap();
    assert!(app.revision_after_checkpoint(&copy, &Ok(())));
    let session = app
        .revision
        .saved
        .as_ref()
        .unwrap()
        .copy
        .target_session_id
        .clone();
    let mut mapped = original;
    mapped.session_id = session.clone();
    mapped.messages[1].content.attachments =
        sources(&session).messages[1].content.attachments.clone();
    app.revision_completed(copy, Ok(Output::Sources(mapped)));
    assert_eq!(app.revision.saved.as_ref().unwrap().stage, Stage::Bindings);
    assert!(app.revision_request().is_none());
    frame(&mut app, 80, 30);
    assert!(!app.revision_enabled(&Command::Send));
    let reselect = app
        .layer
        .rect("reference-review/items/0/actions/reselect")
        .expect("the source review exposes a Select again button");
    assert!(!reselect.is_empty());
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.input(Event::Mouse(MouseEvent {
            kind,
            column: reselect.x,
            row: reselect.y,
            modifiers: KeyModifiers::NONE,
        }));
    }
    let query = app.completion_request().unwrap();
    let mut fresh = old.clone();
    fresh.session_id = session.clone();
    fresh.activation = "new".into();
    fresh.registration = uuid::Uuid::new_v4();
    let provider = InputResourceProjection {
        provider: "records".into(),
        package_id: "records".into(),
        scope_id: maka_runtime::scope::Scope::Session(session.clone()),
        method: "resources".into(),
        target: maka_plugins::remote::Target {
            entry_id: fresh.entry_id.clone(),
            activation: fresh.activation.clone(),
            registration: fresh.registration,
        },
        descriptor: maka_plugins::input::resources::Descriptor {
            title: maka_plugins::terminal_ui::Text::plain("Records"),
        },
    };
    app.completion_completed(
        query,
        Ok(completion::Output::Providers(Page {
            items: vec![provider],
            next_cursor: None,
        })),
    );
    let resolve = app.completion_request().unwrap();
    let binding = Binding {
        label: "Record seven".into(),
        origin: Origin::Plugin {
            title: "Records".into(),
            package: "records".into(),
        },
        inline: None,
        payload: Payload::Selection {
            provider: "records".into(),
            selector: "7".into(),
            source: Some(fresh.clone()),
            quote: None,
        },
    };
    app.completion_completed(resolve, Ok(completion::Output::Captured(binding)));
    assert_eq!(
        app.revision.saved.as_ref().unwrap().inputs[0]
            .message()
            .input_selection_sources,
        vec![old.clone()]
    );
    assert!(app.revision_request().is_none());
    // Even an immediate Enter cannot accept the unpresented resolved source.
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(
        app.revision.saved.as_ref().unwrap().inputs[0]
            .resolved
            .is_empty()
    );
    frame(&mut app, 80, 30);
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(!app.completion_open());
    assert_eq!(
        app.revision.saved.as_ref().unwrap().inputs[0]
            .original
            .input_selection_sources,
        vec![old]
    );
    assert_eq!(
        app.revision.saved.as_ref().unwrap().inputs[0]
            .message()
            .input_selection_sources,
        vec![fresh.clone()]
    );
    assert!(
        app.revision_request().is_none(),
        "accepting a reference is not a batch confirmation"
    );
    app.revision.checkpoint().unwrap().validate("root").unwrap();
    frame(&mut app, 80, 30);
    assert!(app.revision_enabled(&Command::Send));
    app.apply(Action::Revision(Command::Send));
    let start = app.revision_request().unwrap();
    let revision::Job::Start(batch) = start.job else {
        panic!("expected batch only after confirmation")
    };
    assert_eq!(batch.session_id, session);
    assert_eq!(batch.messages[0].input_selection_sources, vec![fresh]);
}

#[test]
fn a_revision_slash_command_consumes_only_its_token_in_the_canonical_input() {
    let (mut app, basis) = crate::pages::branch::tests::fixture();
    app.apply(Action::Revision(Command::Open(basis)));
    let load = app.revision_request().unwrap();
    app.revision_completed(load, Ok(Output::Sources(sources("source"))));
    frame(&mut app, 80, 30);
    for ch in "/help".chars() {
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::NONE,
        )));
    }
    frame(&mut app, 80, 30);
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(app.help);
    let input = &app.revision.saved.as_ref().unwrap().inputs[0];
    assert_eq!(
        input.content.text,
        sources("source").messages[0].content.text
    );
    assert_eq!(
        input.content.inline_references.as_ref().unwrap()[0].start,
        3
    );
    app.revision.checkpoint().unwrap().validate("root").unwrap();
}
