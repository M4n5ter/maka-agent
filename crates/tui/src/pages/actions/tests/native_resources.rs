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
use crate::pages::{attention, resources};

#[test]
fn resource_write_requires_visible_surface_and_original_epoch_through_checkpoint() {
    let mut app = session(Locale::En);
    let terminal = app
        .resources_action()
        .expect("ordinary conversation exposes Terminal");
    assert!(
        session_commands(&app)
            .iter()
            .any(|(action, _)| action == &terminal)
    );
    assert!(
        session_commands(&app)
            .iter()
            .any(|(action, _)| matches!(action, Action::SessionControls(_)))
    );
    app.apply(terminal);
    let launch = Action::Resources(resources::Command::NewTerminal);
    assert!(
        !app.enabled(&launch),
        "unpainted surface cannot launch a process"
    );
    draw(&mut app, 80);
    assert!(app.enabled(&launch));
    app.apply(launch);
    let request = app.resources.request().unwrap();
    assert!(request.needs_checkpoint());
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "replacement".into(),
    };
    assert!(!app.resources_after_checkpoint(&request, &Ok(())));
    assert!(
        app.resources.checkpoint().is_some(),
        "uncertain identity remains queryable"
    );
    assert!(
        app.resources.request().is_none(),
        "epoch changes never replay a launch"
    );
    app.reconcile_resources();
    assert!(!app.resources.visible);
}

#[test]
fn global_notification_entry_remains_reachable_at_narrow_width_without_palette() {
    for locale in Locale::ALL {
        for width in [30, 48, 100] {
            let mut app = session(locale);
            draw(&mut app, width);
            let badge = app.chrome.header.rect("header/right/attention").unwrap();
            assert!(badge.width >= 5 && badge.height > 0);
            app.input(click(badge));
            assert_eq!(app.overlay(), Some(crate::overlay::Overlay::Attention));
            draw(&mut app, width);
            assert!(app.attention_presented, "locale {locale:?}, width {width}");
            assert!(app.palette.is_none());
            app.apply(Action::Attention(attention::Command::Close));
            assert!(!app.attention.visible);
        }
    }
}

#[test]
fn a_full_height_object_menu_cannot_acknowledge_the_badge_it_covers() {
    let mut app = session(Locale::En);
    let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
    terminal
        .draw(|frame| crate::view::draw(frame, &mut app))
        .unwrap();
    assert!(app.attention_badge_visible());
    let menu = app
        .chrome
        .header
        .rect("header/right/session-actions")
        .unwrap();
    app.input(click(menu));
    terminal
        .draw(|frame| crate::view::draw(frame, &mut app))
        .unwrap();
    assert!(app.overlay().is_none());
    assert!(app.chrome.header.captures());
    assert!(
        !app.attention_badge_visible(),
        "a covered badge is not a delivered notification"
    );
    app.input(key(KeyCode::Esc));
    terminal
        .draw(|frame| crate::view::draw(frame, &mut app))
        .unwrap();
    assert!(app.attention_badge_visible());
}
