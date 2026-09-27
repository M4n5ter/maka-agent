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
use serde_json::json;

fn app() -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    app.apply(Action::Visit(Route::Session("source".into())));
    app
}
fn draw(app: &mut App, width: u16, height: u16) {
    Terminal::new(TestBackend::new(width, height))
        .unwrap()
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
}
fn command(app: &mut App, command: Command) {
    app.apply(Action::Bundle(command));
}
fn enter(app: &mut App) {
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
}
fn click(app: &mut App, path: &str) {
    let rect = app.layer.rect(path).expect("visible control");
    app.input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }));
}
fn exported_preview(app: &mut App) {
    app.apply(app.bundle_export_action().unwrap());
    let request = app.bundle_request().unwrap();
    assert!(!request.needs_checkpoint());
    app.bundle_completed(
        request,
        Ok(Output::Preview(api::Previewed {
            session_count: 3,
            subtree_digest: "a".repeat(64),
        })),
    );
}
fn import_preview() -> api::ImportPreviewed {
    api::ImportPreviewed {
        bundle_digest: format!("sha256:{}", "a".repeat(64)),
        binding_digest: format!("sha256:{}", "b".repeat(64)),
        session_count: 2,
        artifact_files: 7,
        resolved_workspace: maka_protocol::session::WorkspaceProjection {
            target: WorkspaceTarget::HostPath {
                path: "/host/project".into(),
            },
            host_cwd: "/host/project".into(),
        },
    }
}
fn importing(app: &mut App) {
    app.apply(app.bundle_import_action().unwrap());
    app.bundle.path.insert("/host/history.maka-session");
    app.bundle.workspace.insert("/host/project");
    draw(app, 100, 35);
    command(app, Command::Review);
    let preview = app.bundle_request().unwrap();
    assert!(!preview.needs_checkpoint());
    assert!(matches!(&preview.kind, RequestKind::ImportPreview(input)
        if input.source == "/host/history.maka-session"
            && input.workspace == WorkspaceTarget::HostPath { path: "/host/project".into() }));
    app.bundle_completed(preview, Ok(Output::ImportPreview(import_preview())));
    draw(app, 100, 35);
}

#[test]
fn export_confirmation_freezes_subtree_and_destination_and_never_replays_unknown() {
    let mut app = app();
    exported_preview(&mut app);
    app.bundle.path.insert("/host/export.maka-session");
    command(&mut app, Command::Review);
    assert!(!app.bundle.reviewing, "hidden controls cannot confirm");
    draw(&mut app, 100, 35);
    command(&mut app, Command::Review);
    draw(&mut app, 100, 35);
    enter(&mut app);
    assert!(!app.bundle.visible, "confirmation opens on Close");
    assert!(app.bundle_request().is_none());
    command(&mut app, Command::Resume);
    draw(&mut app, 100, 35);
    click(&mut app, "footer/confirm");
    let request = app.bundle_request().unwrap();
    assert!(
        matches!(&request.kind, RequestKind::Write(Frozen::Export { session, destination, digest, count })
        if session == "source" && destination == "/host/export.maka-session" && digest == &"a".repeat(64) && *count == 3)
    );
    let saved = app.bundle.checkpoint().unwrap();
    saved.validate("root").unwrap();
    assert!(saved.validate("other-root").is_err());
    assert!(app.bundle_after_checkpoint(&request, &Ok(())));
    assert!(
        !app.bundle_after_checkpoint(&request, &Ok(())),
        "one dispatch per checkpoint"
    );
    app.bundle_completed(
        request.clone(),
        Err(RequestFailure::Unknown(ClientError::Timeout)),
    );
    assert!(!app.bundle_enabled(&Command::Confirm));
    assert!(
        !app.bundle_enabled(&Command::Query),
        "export cannot manufacture an original receipt"
    );
    let mut restored = State::default();
    restored.restore(saved);
    app.bundle = restored;
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "new".into(),
    };
    command(&mut app, Command::Resume);
    draw(&mut app, 100, 35);
    assert!(
        app.bundle_request().is_none(),
        "resume does not replay writes"
    );
    app.bundle_completed(
        request,
        Ok(Output::Exported(api::Exported {
            session_count: 3,
            compressed_bytes: 123,
        })),
    );
    assert!(
        matches!(app.bundle.outcome, Outcome::Unknown { .. }),
        "old epoch cannot settle restored request"
    );
    command(&mut app, Command::Forget);
    draw(&mut app, 100, 35);
    enter(&mut app);
    assert!(
        app.bundle.intent.is_some(),
        "forget confirmation defaults to keep"
    );
}

#[test]
fn import_unknown_recovers_only_original_content_binding_and_rejects_late_reads() {
    let mut app = app();
    importing(&mut app);
    command(&mut app, Command::Confirm);
    let write = app.bundle_request().unwrap();
    let saved = app.bundle.checkpoint().unwrap();
    assert!(app.bundle_after_checkpoint(&write, &Ok(())));
    app.bundle.disconnect();
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "new".into(),
    };
    app.bundle.restore(saved);
    command(&mut app, Command::Resume);
    draw(&mut app, 100, 35);
    assert!(app.bundle_request().is_none());
    command(&mut app, Command::Query);
    let query = app.bundle_request().unwrap();
    assert!(!query.needs_checkpoint());
    assert!(
        matches!(&query.kind, RequestKind::Query(Frozen::Import { expected, .. })
        if *expected == import_preview())
    );
    app.bundle_completed(
        query.clone(),
        Ok(Output::Queried(api::ImportQueried { receipt: None })),
    );
    assert!(matches!(app.bundle.outcome, Outcome::Unknown { .. }));
    assert!(!app.bundle_enabled(&Command::Confirm));
    draw(&mut app, 100, 35);
    command(&mut app, Command::Query);
    let retry = app.bundle_request().unwrap();
    let receipt = || api::ImportQueried {
        receipt: Some(api::ImportReceipt {
            root_session_id: "imported-root".into(),
            session_ids: vec!["imported-child".into(), "imported-root".into()],
        }),
    };
    app.bundle_completed(query, Ok(Output::Queried(receipt())));
    assert_eq!(app.bundle.pending.as_ref(), Some(&retry));
    app.bundle_completed(retry, Ok(Output::Queried(receipt())));
    assert!(matches!(
        &app.bundle.outcome,
        Outcome::Complete {
            receipt: Receipt::Imported {
                artifacts: None,
                ..
            },
            ..
        }
    ));
    app.bundle.checkpoint().unwrap().validate("root").unwrap();
    assert_eq!(
        app.navigation.current(),
        Route::Session("source".into()),
        "late success does not navigate"
    );
    draw(&mut app, 100, 35);
    command(&mut app, Command::Visit);
    assert_eq!(
        app.navigation.current(),
        Route::Session("imported-root".into())
    );
}

#[test]
fn failed_checkpoint_and_rejected_writes_preserve_editable_draft_but_postpublish_failures_do_not() {
    for code in [
        None,
        Some(OperationErrorCode::CandidateSetStale),
        Some(OperationErrorCode::PersistenceFailed),
        Some(OperationErrorCode::InternalFailure),
        Some(OperationErrorCode::CommitOutcomeUnknown),
    ] {
        let mut app = app();
        importing(&mut app);
        command(&mut app, Command::Confirm);
        let request = app.bundle_request().unwrap();
        if let Some(code) = code {
            assert!(app.bundle_after_checkpoint(&request, &Ok(())));
            app.bundle_completed(
                request,
                Err(RequestFailure::Rejected(ClientError::Rejected(
                    maka_protocol::OperationError {
                        code,
                        message: "fixture failure".into(),
                    },
                ))),
            );
        } else {
            assert!(!app.bundle_after_checkpoint(&request, &Err("disk unavailable".into())));
        }
        let unknown = code.is_some_and(|code| code != OperationErrorCode::CandidateSetStale);
        assert_eq!(
            matches!(app.bundle.outcome, Outcome::Unknown { .. }),
            unknown
        );
        assert_eq!(app.bundle.path.text(), "/host/history.maka-session");
        assert!(app.bundle_request().is_none());
    }
}

#[test]
fn draft_fields_pointer_keyboard_and_narrow_confirmation_remain_owned_in_three_locales() {
    for locale in Locale::ALL {
        for (width, height) in [(80, 24), (44, 22)] {
            let mut app = app();
            app.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
            app.apply(app.bundle_import_action().unwrap());
            draw(&mut app, width, height);
            assert!(
                app.bundle.rendered,
                "import form must fit {width}×{height} {locale:?}"
            );
            app.input(Event::Paste("/host/会话 🦀.maka-session".into()));
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
            draw(&mut app, width, height);
            click(&mut app, "workspace/input");
            app.input(Event::Paste("/host/工作区".into()));
            assert_eq!(app.bundle.workspace.text(), "/host/工作区");
            app.input(Event::Paste("\n/unsafe".into()));
            assert_eq!(
                app.bundle.workspace.text(),
                "/host/工作区",
                "paste never executes or adds controls"
            );
            app.bundle.checkpoint().unwrap().validate("root").unwrap();
            command(&mut app, Command::Review);
            let request = app
                .bundle_request()
                .expect("visible valid form can request preview");
            let mut preview = import_preview();
            preview.resolved_workspace = maka_protocol::session::WorkspaceProjection {
                target: WorkspaceTarget::HostPath {
                    path: "/host/工作区".into(),
                },
                host_cwd: "/host/工作区".into(),
            };
            app.bundle_completed(request, Ok(Output::ImportPreview(preview)));
            draw(&mut app, width, height);
            assert!(
                app.bundle_enabled(&Command::Confirm),
                "review must fit {width}×{height} {locale:?}"
            );
            assert_eq!(app.layer.focused_path(), Some("footer/close"));
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
            assert_eq!(app.layer.focused_path(), Some("footer/confirm"));
            // Dismissing while Confirm holds focus must not make the reopened
            // review default to writing. The original draft/preview are retained.
            app.input(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }));
            assert!(!app.bundle.visible);
            command(&mut app, Command::Resume);
            draw(&mut app, width, height);
            assert_eq!(app.layer.focused_path(), Some("footer/close"));
            assert!(app.bundle_request().is_none());
            draw(&mut app, 28, 8);
            assert!(
                !app.bundle_enabled(&Command::Confirm),
                "hidden confirmations are disabled"
            );
            draw(&mut app, width, height);
            assert!(app.bundle_enabled(&Command::Confirm));
        }
    }
}

#[test]
fn export_form_and_unknown_import_recovery_fit_common_and_narrow_frames() {
    for locale in Locale::ALL {
        for (width, height) in [(80, 24), (44, 22)] {
            let mut export = app();
            export.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
            exported_preview(&mut export);
            export
                .bundle
                .path
                .insert("/host/long destination/会话 🦀.maka-session");
            draw(&mut export, width, height);
            assert!(
                export.bundle.rendered,
                "export form must fit {width}×{height} {locale:?}"
            );
            assert!(export.layer.rect("draft-actions/preview").is_some());
            assert!(export.layer.rect("draft-actions/discard").is_some());
            click(&mut export, "footer/review");
            draw(&mut export, width, height);
            assert_eq!(export.layer.focused_path(), Some("footer/close"));
            enter(&mut export);
            assert!(!export.bundle.visible);
            assert!(export.bundle_request().is_none());

            let mut import = app();
            importing(&mut import);
            command(&mut import, Command::Confirm);
            let request = import.bundle_request().unwrap();
            assert!(import.bundle_after_checkpoint(&request, &Ok(())));
            import.bundle_completed(request, Err(RequestFailure::Unknown(ClientError::Timeout)));
            import.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
            draw(&mut import, width, height);
            assert!(
                import.bundle.rendered,
                "unknown recovery must fit {width}×{height} {locale:?}"
            );
            assert_eq!(import.layer.focused_path(), Some("footer/close"));
            assert!(import.layer.rect("footer/query").is_some());
            click(&mut import, "footer/query");
            let query = import.bundle_request().unwrap();
            import.bundle_completed(
                query,
                Ok(Output::Queried(api::ImportQueried { receipt: None })),
            );
            draw(&mut import, width, height);
            assert!(
                import.bundle.rendered,
                "a missing-receipt diagnostic must not hide recovery controls"
            );
            assert!(import.bundle_enabled(&Command::Query));
            assert!(!import.bundle_enabled(&Command::Confirm));
        }
    }
}

#[test]
fn checkpoint_rejects_changed_request_fields_and_receipt_kind() {
    let mut app = app();
    importing(&mut app);
    command(&mut app, Command::Confirm);
    let saved = serde_json::to_value(app.bundle.checkpoint().unwrap()).unwrap();
    for (pointer, value) in [
        ("/path", json!("/other/file")),
        ("/workspace", json!("/other/workspace")),
        (
            "/outcome/write/expected/bundleDigest",
            json!("not-a-digest"),
        ),
        (
            "/outcome/write/expected/bindingDigest",
            json!("a".repeat(64)),
        ),
    ] {
        let mut bad = saved.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        let checkpoint: Checkpoint = serde_json::from_value(bad).unwrap();
        assert!(checkpoint.validate("root").is_err(), "{pointer}");
    }
}
