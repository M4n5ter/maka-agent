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

pub(super) fn commands(bench: &mut Bench, samples: &mut Vec<Value>) {
    let tail = format!("/entry-{:05}", bench.inventory - 1);
    for (phase, base, suffix) in [
        ("slash_broad", "/entry".to_owned(), '-'),
        (
            "slash_tail",
            tail[..tail.len() - 1].into(),
            tail.chars().last().unwrap(),
        ),
        ("slash_no_match", "/unmatched".into(), 'x'),
    ] {
        bench.text(&base);
        for index in 0..WARMUP + MEASURED {
            let event = key(if index.is_multiple_of(2) {
                KeyCode::Char(suffix)
            } else {
                KeyCode::Backspace
            });
            samples.push(bench.sample(phase, index, "input", |app| {
                app.input(event);
            }));
            assert!(bench.app.completion_open());
            if phase == "slash_no_match" {
                assert!(
                    bench
                        .app
                        .completion
                        .popup
                        .as_ref()
                        .unwrap()
                        .candidates
                        .is_empty()
                );
            }
        }
    }
}

pub(super) fn references(bench: &mut Bench, samples: &mut Vec<Value>) {
    bench.text("@");
    let pages = bench.inventory.div_ceil(io::PAGE);
    for index in 0..WARMUP + MEASURED {
        let page = index % pages;
        if page == 0 {
            bench
                .app
                .completion_action(Command::Category(Category::Workspace));
        } else {
            bench.app.completion_action(Command::Next);
        }
        let request = bench.app.completion_request().unwrap();
        let output = fixtures::workspace_page(&request, bench.inventory, page * io::PAGE);
        samples.push(bench.sample("reference_page", index, "callback", |app| {
            app.completion_completed(request, Ok(output))
        }));
        samples.push(bench.sample("reference_cursor", index, "input", |app| {
            app.input(key(KeyCode::Down));
        }));
        assert_eq!(
            bench
                .app
                .completion
                .popup
                .as_ref()
                .unwrap()
                .candidates
                .len(),
            io::PAGE.min(bench.inventory - page * io::PAGE)
        );
    }
}

pub(super) fn pending(bench: &mut Bench, samples: &mut Vec<Value>) {
    bench.text("@");
    bench
        .app
        .completion_action(Command::Category(Category::Plugins));
    let catalog = bench.app.completion_request().unwrap();
    bench.app.completion_completed(
        catalog,
        Ok(Output::Providers(maka_protocol::plugin::Page {
            items: vec![fixtures::provider()],
            next_cursor: None,
        })),
    );
    bench.draw();
    let popup = bench.app.completion.popup.as_ref().unwrap();
    let choose = Command::Choose {
        generation: popup.generation,
        id: popup.selected.clone().unwrap(),
    };
    bench.app.completion_action(choose);
    let held = bench.app.completion_request().unwrap();
    assert!(matches!(held.job, io::Job::Resources { .. }));
    for index in 0..WARMUP + MEASURED {
        let event = key(if index.is_multiple_of(2) {
            KeyCode::Char('x')
        } else {
            KeyCode::Backspace
        });
        samples.push(bench.sample("pending_query_edit", index, "input", |app| {
            app.input(event);
        }));
        assert!(held.cancel.cancelled());
        assert!(
            bench.app.completion_request().is_none(),
            "query changes coalesce until the original cleanup completes"
        );
        assert_eq!(bench.app.completion.pending.as_ref().unwrap().id, held.id);
    }
    // Settle the edited-away read, then retire a still-current request. This
    // keeps catalog retirement distinct from the earlier typing cancellation.
    bench
        .app
        .completion_completed(held, Err("controlled cleanup settled".into()));
    let retired = bench.app.completion_request().unwrap();
    assert!(!retired.cancel.cancelled());
    // Descriptive one-off transitions, not 200-sample distributions.
    samples.push(bench.sample(
        "pending_provider_retirement",
        MEASURED,
        "callback",
        App::completion_catalog_changed,
    ));
    assert!(retired.cancel.cancelled());
    let stale = fixtures::stale_resources();
    samples.push(
        bench.sample("retired_provider_reply", MEASURED, "callback", |app| {
            app.completion_completed(retired, Ok(stale))
        }),
    );
    assert!(
        bench
            .app
            .completion
            .popup
            .as_ref()
            .unwrap()
            .candidates
            .is_empty()
    );
    assert!(bench.app.completion.pending.is_none());
    samples.push(
        bench.sample("pending_query_close", MEASURED, "input", |app| {
            app.input(key(KeyCode::Esc));
        }),
    );
    assert!(!bench.app.completion_open());
}

pub(super) fn retirement(bench: &mut Bench, samples: &mut Vec<Value>) {
    bench.text(&format!("/entry-{:05}", bench.inventory - 1));
    let directory = bench.app.apps.directory.clone();
    for index in 0..WARMUP + MEASURED {
        let popup = bench.app.completion.popup.as_ref().unwrap();
        let old = Command::Choose {
            generation: popup.generation,
            id: popup.selected.clone().unwrap(),
        };
        samples.push(
            bench.sample("command_retirement", index, "callback", |app| {
                app.apps.directory.clear();
                app.completion_catalog_changed();
            }),
        );
        assert!(!bench.app.completion_enabled(&old));
        samples.push(
            bench.sample("retired_command_enter", index, "input", |app| {
                app.input(key(KeyCode::Enter));
            }),
        );
        assert_eq!(
            bench.app.navigation.current(),
            Route::Session("chat".into())
        );
        // Re-publishing the fixture happens outside timing; no Host mutation is inferred.
        bench.app.apps.directory = directory.clone();
        bench.app.completion_catalog_changed();
        bench.draw();
    }
}
