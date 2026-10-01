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
    Locale, LocalePreference,
    app::{Action, App, ConnectionState, Focus},
    i18n::I18n,
    navigation::Route,
    theme::Choice,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::time::{Duration, Instant};

fn draft() -> App {
    let mut app = App::new(
        "/unused".into(),
        I18n::new(LocalePreference::Auto, Locale::En),
    );
    app.connection = ConnectionState::Connected {
        root_id: "root".into(),
        epoch: "epoch".into(),
    };
    let input = maka_protocol::session::decode_session_create_input(&serde_json::json!({
        "sessionId":"draft", "workspace":{"kind":"host_path","path":"/tmp"}, "modelTarget":{"kind":"default"}
    })).unwrap();
    app.pending_new.insert("draft".into(), input);
    app.apply(Action::Visit(Route::Session("draft".into())));
    app.focus = Focus::Composer;
    app
}
fn screen(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::view::draw(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn dots(buffer: &Buffer) -> usize {
    buffer
        .content
        .iter()
        .filter(|cell| {
            cell.symbol()
                .chars()
                .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
        })
        .count()
}
#[test]
fn welcome_runs_only_on_idle_drafts_and_never_changes_composer_geometry() {
    let mut app = draft();
    assert!(dots(&screen(&mut app, 120, 42)) > 80);
    assert!(app.chrome.animation.wait(Instant::now()).is_some());
    for (width, height) in [(120, 42), (80, 24), (60, 16), (40, 12)] {
        app.chrome.ascii = false;
        screen(&mut app, width, height);
        let editor = app
            .chrome
            .composer
            .rect("composer/body/content/editor")
            .unwrap();
        let send = app
            .chrome
            .composer
            .rect("composer/body/actions/buttons/send")
            .unwrap();
        assert!(editor.height >= 1 && editor.bottom() <= height - 2 && send.height == 1);
        app.chrome.ascii = true;
        assert_eq!(dots(&screen(&mut app, width, height)), 0);
        assert_eq!(
            app.chrome.composer.rect("composer/body/content/editor"),
            Some(editor)
        );
        assert_eq!(
            app.chrome
                .composer
                .rect("composer/body/actions/buttons/send"),
            Some(send)
        );
    }
    app.chrome.ascii = false;
    screen(&mut app, 120, 42);
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    assert_eq!(app.drafts["draft"].text(), "a");
    assert_eq!(dots(&screen(&mut app, 120, 42)), 0);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE,
    )));
    assert!(dots(&screen(&mut app, 120, 42)) > 80);
    assert!(app.chrome.animation.wait(Instant::now()).is_some());
    app.apply(Action::Palette);
    assert_eq!(dots(&screen(&mut app, 120, 42)), 0);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.input(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    app.input(Event::FocusLost);
    assert!(dots(&screen(&mut app, 120, 42)) > 80);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.input(Event::FocusGained);
    app.chrome.motion = false;
    assert!(dots(&screen(&mut app, 120, 42)) > 80);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.chrome.motion = true;
    assert_eq!(dots(&screen(&mut app, 40, 12)), 0);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.apply(Action::Visit(Route::Workspace));
    screen(&mut app, 120, 42);
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
    app.apply(Action::Visit(Route::Session("draft".into())));
    assert!(dots(&screen(&mut app, 120, 42)) > 80);
    app.pending_new.remove("draft");
    assert_eq!(
        dots(&screen(&mut app, 120, 42)),
        0,
        "remote loading is not a welcome screen"
    );
    assert!(app.chrome.animation.wait(Instant::now()).is_none());
}

#[test]
fn welcome_remains_bounded_and_continues_across_many_cycles() {
    for theme in [Choice::Maka, Choice::Paper, Choice::Terminal] {
        let mut welcome = Welcome::default();
        let mut motion = Motion::default();
        let now = Instant::now();
        let area = Rect::new(0, 0, 90, 40);
        let free = Rect::new(13, 4, 65, 30);
        let mut first = None;
        for ms in [0, 700, 12_000, 43_000, 1_000_000] {
            motion.begin(now + Duration::from_millis(ms), true);
            let mut buffer = Buffer::empty(area);
            welcome.draw(free, &mut buffer, theme.colors(), &mut motion);
            assert!(dots(&buffer) > 20);
            if theme == Choice::Terminal {
                assert!(
                    buffer
                        .content
                        .iter()
                        .filter(|cell| cell.symbol() != " ")
                        .all(|cell| cell.fg == ratatui::style::Color::Rgb(0x47, 0xa3, 0xe2)),
                    "native terminal palettes must not substitute a different brand hue"
                );
            }
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    if !free.contains((x, y).into()) {
                        assert_eq!(buffer[(x, y)].symbol(), " ");
                    }
                }
            }
            if ms == 0 {
                first = Some(buffer.clone());
            }
            if ms == 700 {
                assert_ne!(first.as_ref().unwrap(), &buffer);
            }
            assert!(motion.wait(now + Duration::from_millis(ms)).is_some());
        }
        motion.begin(now, false);
        welcome.draw(free, &mut Buffer::empty(area), theme.colors(), &mut motion);
        assert!(motion.wait(now).is_none());
        motion.begin(now, true); // nothing visible requested a frame
        assert!(motion.wait(now).is_none());
    }
}

#[test]
fn formations_hold_readable_poses_and_move_through_intermediate_positions() {
    let area = Rect::new(0, 0, 66, 22);
    let mut renderer = super::render::Renderer::default();
    let mut render = |phase| {
        let mut buffer = Buffer::empty(area);
        renderer.draw(area, &mut buffer, Choice::Maka.colors(), Some(phase));
        buffer
    };
    let logo = render(0.0);
    let word = render(0.48);
    let transition = render(0.24);
    let bounds = |buffer: &Buffer| {
        let mut points = buffer
            .content
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.symbol() != " ")
            .map(|(i, _)| ((i % 66) as u16, (i / 66) as u16));
        let (x, y) = points.next().unwrap();
        let (left, top, right, bottom) = points.fold((x, y, x, y), |(l, t, r, b), (x, y)| {
            (l.min(x), t.min(y), r.max(x), b.max(y))
        });
        (right - left + 1, bottom - top + 1)
    };
    let (logo_width, logo_height) = bounds(&logo);
    let (word_width, word_height) = bounds(&word);
    assert!(word_width > logo_width * 3 / 2 && word_height < logo_height);
    assert_ne!(transition, logo);
    assert_ne!(transition, word);
    let closing = render(0.99999);
    let changed = logo
        .content
        .iter()
        .zip(&closing.content)
        .filter(|(a, b)| a.symbol() != b.symbol())
        .count();
    assert!(
        changed < 8,
        "the loop must close without a visible shape jump: {changed}"
    );
}
