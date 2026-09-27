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

#[test]
fn export_reviews_exact_installed_bytes_and_saves_original_host_destination_before_dispatch() {
    let mut app = app(Place::Export("example.board".into()));
    app.plugins.draft_mut().unwrap().fields[0] = drafts::editor("/host/board.maka-extension", 4096);
    let token = review(&mut app, Change::Export);
    app.plugins.draft_mut().unwrap().fields[0] = drafts::editor("/host/later", 4096);
    app.plugins.snapshot.as_mut().unwrap().packages[0].content_digest =
        format!("sha256-{}", "b".repeat(64));
    app.apply(Action::Plugins(Command::Confirm(token)));
    let request = app.plugins_request().unwrap();
    let io::Mutation::Export(input) = request.mutation().unwrap() else {
        panic!("export write")
    };
    assert_eq!(input.target_path, "/host/board.maka-extension");
    assert_eq!(input.expected.base_generation, 7);
    assert_eq!(
        input.expected.content_digest,
        Some(format!("sha256-{}", "a".repeat(64)))
    );
    let saved = app.plugins.checkpoint();
    saved.validate("root").unwrap();
    let encoded = serde_json::to_value(&saved).unwrap();
    assert_eq!(encoded["pending"][0]["export_target"], input.target_path);
    assert_eq!(
        encoded["pending"][0]["package_digest"].as_str(),
        input.expected.content_digest.as_deref()
    );
    assert!(app.plugins_after_checkpoint(&request, &Ok(())));
    assert!(!app.plugins_after_checkpoint(&request, &Ok(())));
    app.plugins_completed(request, Err(RequestFailure::Unknown(ClientError::Timeout)));
    assert!(
        app.plugins
            .export_uncertain("example.board", "/host/board.maka-extension")
    );
    let mut restored = State::default();
    restored.restore(saved);
    assert_eq!(restored.unknown.len(), 1);
    assert!(restored.queued.is_none() && restored.pending.is_none());
    let mut reopened = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
    );
    reopened.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "new-epoch".into(),
    };
    reopened.plugins = restored;
    reopened.apply(Action::Visit(Route::Plugins(Place::Export(
        "example.board".into(),
    ))));
    let mut app = reopened;
    let read = app.plugins_request().unwrap();
    assert!(!read.needs_checkpoint());
    app.plugins_completed(read, Ok(Output::Snapshot(snapshot())));
    app.plugins.draft_mut().unwrap().fields[0] = drafts::editor("/host/board.maka-extension", 4096);
    draw(&mut app, 100, 35);
    app.apply(Action::Plugins(Command::Review(
        app.plugins.token,
        Change::Export,
    )));
    assert!(
        app.plugins.confirmation.is_none(),
        "an unresolved target is not replayed"
    );
    assert!(app.plugins_request().is_none());
    app.plugins.draft_mut().unwrap().fields[0] = drafts::editor("/host/another-file", 4096);
    draw(&mut app, 100, 35);
    app.apply(Action::Plugins(Command::Review(
        app.plugins.token,
        Change::Export,
    )));
    assert!(
        app.plugins.confirmation.is_some(),
        "another destination requires a fresh explicit review"
    );
}

#[test]
fn export_field_and_review_remain_usable_in_three_locales_without_executing_on_open_or_resize() {
    for locale in Locale::ALL {
        let mut app = app(Place::Export("example.board".into()));
        app.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
        for (width, height) in [(100, 35), (52, 24)] {
            draw(&mut app, width, height);
            assert!(
                app.plugins
                    .surface
                    .rect("plugins/scroll/body/field-0")
                    .is_some()
            );
            assert!(app.plugins_request().is_none());
        }
        app.plugins.draft_mut().unwrap().fields[0] =
            drafts::editor("/host/插件 🦀.maka-extension", 4096);
        let token = review(&mut app, Change::Export);
        draw(&mut app, 52, 24);
        assert!(app.plugins_enabled(&Command::Confirm(token)));
        draw(&mut app, 25, 7);
        assert!(!app.plugins_enabled(&Command::Confirm(token)));
        draw(&mut app, 100, 35);
        app.apply(Action::Plugins(Command::Confirm(token)));
        let request = app.plugins_request().unwrap();
        assert!(!app.plugins_after_checkpoint(&request, &Err("offline state store".into())));
        assert!(app.plugins.unknown.is_empty());
        assert_eq!(
            app.plugins.draft().unwrap().fields[0].text(),
            "/host/插件 🦀.maka-extension"
        );
    }
}

#[test]
fn export_success_is_a_file_result_and_safe_drafts_survive_while_opaque_payloads_stay_withheld() {
    let mut app = app(Place::Export("example.board".into()));
    app.plugins.draft_mut().unwrap().fields[0] = drafts::editor("/host/board.maka-extension", 4096);
    app.plugins.draft_mut().unwrap().dirty[0] = true;
    let request = confirm(&mut app, Change::Export);
    assert!(app.plugins_after_checkpoint(&request, &Ok(())));
    app.plugins_completed(
        request,
        Ok(Output::Exported(maka_protocol::plugin::Exported {
            target_path: "/host/board.maka-extension".into(),
        })),
    );
    assert!(app.plugins.receipt.is_none());
    let checkpoint = app.plugins.checkpoint();
    checkpoint.validate("root").unwrap();
    let encoded = serde_json::to_value(&checkpoint).unwrap();
    assert!(encoded["pending"].as_array().unwrap().is_empty());
    assert!(encoded["withheld"].as_array().unwrap().is_empty());
    assert_eq!(encoded["exported"]["target"], "/host/board.maka-extension");
    let mut restored = State::default();
    restored.restore(checkpoint);
    assert_eq!(
        restored.exported.as_ref().unwrap().target,
        "/host/board.maka-extension"
    );
    assert!(restored.pending.is_none() && restored.queued.is_none());
}
