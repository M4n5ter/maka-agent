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
    app::{Action, ConnectionState},
    i18n::{I18n, Locale},
    pages::completion::{Binding, Checkpoint, DraftKey, Origin, Payload, bindings::SavedDraft},
};

fn app() -> App {
    App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
    )
}

#[test]
fn restored_inline_context_keeps_its_original_source_and_rejects_missing_payloads() {
    let mut original = app();
    original.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    original.apply(Action::Visit(Route::Session("chat".into())));
    assert!(
        original
            .drafts
            .get_mut("chat")
            .unwrap()
            .insert_marked("@Record seven", "binding")
    );
    let source = maka_runtime::input::SelectionSource {
        provider: "example.context".into(),
        package_id: "example".into(),
        entry_id: "example".into(),
        activation: "original".into(),
        registration: uuid::Uuid::new_v4(),
        session_id: "chat".into(),
    };
    original.completion.restore(Checkpoint {
        root: "root".into(),
        drafts: vec![SavedDraft {
            key: DraftKey {
                session: "chat".into(),
                input: None,
                display: false,
            },
            bindings: BTreeMap::from([(
                "binding".into(),
                Binding {
                    label: "Record seven".into(),
                    inline: None,
                    origin: Origin::Plugin {
                        title: "Records".into(),
                        package: "example".into(),
                    },
                    payload: Payload::Selection {
                        provider: "example.context".into(),
                        selector: "record:7".into(),
                        source: Some(source),
                        quote: None,
                    },
                },
            )]),
        }],
    });
    let saved = serde_json::to_value(Snapshot::capture(&original, "root")).unwrap();
    let mut restored = app();
    serde_json::from_value::<Snapshot>(saved.clone())
        .unwrap()
        .restore(&mut restored, false)
        .unwrap();
    let roundtrip = serde_json::to_value(Snapshot::capture(&restored, "root")).unwrap();
    assert_eq!(roundtrip["completion"], saved["completion"]);
    assert_eq!(
        restored.drafts["chat"].marks(),
        original.drafts["chat"].marks()
    );

    let mut missing = saved.clone();
    missing["completion"] = serde_json::Value::Null;
    assert!(
        serde_json::from_value::<Snapshot>(missing)
            .unwrap()
            .validate("root")
            .is_err()
    );
    let mut foreign = saved;
    foreign["completion"]["drafts"][0]["bindings"]["binding"]["payload"]["source"]["sessionId"] =
        "other".into();
    assert!(
        serde_json::from_value::<Snapshot>(foreign)
            .unwrap()
            .validate("root")
            .is_err()
    );
}
