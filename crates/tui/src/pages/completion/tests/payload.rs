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
use crate::pages::completion::model::{Origin, Payload};
use maka_protocol::turn::MessageContent;
use maka_runtime::input::{InlineReferenceKind, QuoteRef, SelectionSource, Selections};
use std::collections::BTreeMap;

pub(super) fn workspace(text: &str) -> Binding {
    Binding {
        label: "中文.rs".into(),
        origin: Origin::Workspace,
        inline: Some(InlineReferenceKind::WorkspaceFile),
        payload: Payload::Context {
            quote: QuoteRef {
                text: text.into(),
                label: Some("中文.rs".into()),
                source_turn_id: None,
                source: None,
            },
            directory: None,
        },
    }
}
fn content(text: &str) -> MessageContent {
    MessageContent {
        text: text.into(),
        display_text: None,
        attachments: None,
        directory_references: None,
        quotes: None,
        inline_references: None,
    }
}
fn merged(app: &App) -> (MessageContent, Selections, Vec<SelectionSource>) {
    let key = app.completion_draft().unwrap();
    let editor = app.completion_editor(&key).unwrap();
    let mut content = content(editor.text());
    let mut selections = Selections::new();
    let mut sources = vec![];
    app.completion_content(&key, editor, &mut content, &mut selections, &mut sources)
        .unwrap();
    (content, selections, sources)
}

#[test]
fn captured_payload_and_utf16_positions_follow_undo_and_redo() {
    let mut app = app(Locale::En);
    app.drafts.get_mut("chat").unwrap().insert("🦀 ");
    let context = app.completion_context(Some(Kind::Reference)).unwrap();
    app.completion_bind(context, workspace("真实正文"));
    let (message, _, _) = merged(&app);
    assert_eq!(message.quotes.unwrap()[0].text, "真实正文");
    assert_eq!(message.inline_references.unwrap()[0].start, 3);
    let before = app.drafts["chat"].marks().to_vec();
    app.drafts
        .get_mut("chat")
        .unwrap()
        .key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    app.completion_gc();
    assert!(merged(&app).0.quotes.is_none());
    app.drafts
        .get_mut("chat")
        .unwrap()
        .key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(app.drafts["chat"].marks(), before);
    assert_eq!(merged(&app).0.quotes.unwrap()[0].text, "真实正文");
    let key = app.completion_draft().unwrap();
    app.completion_bindings_mut(&key)
        .unwrap()
        .remove(&before[0].id);
    assert!(
        app.completion_content(
            &key,
            &app.drafts["chat"],
            &mut content("x"),
            &mut Selections::new(),
            &mut vec![]
        )
        .is_err()
    );
}

#[test]
fn source_identity_survives_same_visible_text_reselection_and_undo() {
    let mut app = app(Locale::En);
    let source = SelectionSource {
        provider: "records".into(),
        package_id: "records".into(),
        entry_id: "records".into(),
        activation: "old".into(),
        registration: uuid::Uuid::new_v4(),
        session_id: "chat".into(),
    };
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
            source: Some(source.clone()),
            quote: None,
        },
    };
    let context = app.completion_context(Some(Kind::Reference)).unwrap();
    app.completion_bind(context, binding.clone());
    let draft = app.completion_draft().unwrap();
    let old_id = app.drafts["chat"].marks()[0].id.clone();
    let old_text = app.drafts["chat"].text().to_owned();
    app.completion_reselect(SelectionTarget {
        draft,
        key: ReferenceKey::Mark(old_id.clone()),
    });
    let mut replacement = binding;
    let Payload::Selection {
        source: Some(next), ..
    } = &mut replacement.payload
    else {
        unreachable!()
    };
    next.activation = "new".into();
    next.registration = uuid::Uuid::new_v4();
    let next = next.clone();
    app.completion_accept_reselection(replacement);
    assert_eq!(app.drafts["chat"].text(), old_text);
    assert_ne!(app.drafts["chat"].marks()[0].id, old_id);
    assert_eq!(merged(&app).2, vec![next]);
    app.drafts
        .get_mut("chat")
        .unwrap()
        .key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_eq!(merged(&app).2, vec![source]);
}

#[test]
fn payload_pressure_trims_history_but_never_discards_a_live_reference() {
    let mut app = app(Locale::En);
    let key = app.completion_draft().unwrap();
    let mut large = workspace(&"中".repeat(20_000));
    large.inline = None;
    for index in 0..80 {
        let id = format!("binding-{index}");
        let editor = app.drafts.get_mut("chat").unwrap();
        editor.insert_marked("@context", &id);
        if index < 79 {
            editor.remove_mark(&id);
        }
        app.completion
            .bindings
            .entry(key.clone())
            .or_default()
            .insert(id, large.clone());
    }
    let text = app.drafts["chat"].text().to_owned();
    assert!(app.completion_retained_bytes() > bindings::BUDGET);
    assert!(app.completion_make_room(&large));
    assert_eq!(app.drafts["chat"].text(), text);
    assert_eq!(app.drafts["chat"].marks().len(), 1);
    assert_eq!(app.completion.bindings[&key].len(), 1);
    assert!(matches!(
        app.notice,
        Some(crate::app::Notice::Local("completion-history-trimmed"))
    ));
    let oversized = BTreeMap::from([(
        String::from("binding-79"),
        Binding {
            payload: Payload::Context {
                quote: QuoteRef {
                    text: "x".repeat(bindings::BUDGET),
                    label: None,
                    source_turn_id: None,
                    source: None,
                },
                directory: None,
            },
            ..large.clone()
        },
    )]);
    app.completion.bindings.insert(key, oversized);
    assert!(!app.completion_make_room(&large));
    assert_eq!(app.drafts["chat"].text(), text);
}
