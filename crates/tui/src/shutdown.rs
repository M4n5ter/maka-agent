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
    app::{Action, App},
    ui::{Role, Sheet, Tone},
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(crate) fn ctrl_c(event: &crossterm::event::Event) -> bool {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    matches!(event, Event::Key(key) if key.kind == KeyEventKind::Press
        && key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('c' | 'C')))
}

/// A frozen local Host lifetime. Never stop a replacement discovered later.
#[derive(Clone)]
pub struct ShutdownRequest {
    pub root: PathBuf,
    pub identity: maka_client::HostIdentity,
    pub interrupt: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Stopped,
    Busy,
}
#[derive(Default)]
pub(crate) struct State {
    pub stopping: bool,
    pub prompt: Option<Prompt>,
    pub quit_deadline: Option<Instant>,
}
pub(crate) enum Prompt {
    Busy,
    Failed(String),
}
impl State {
    pub fn quit_press(&mut self, previous: Option<Instant>, now: Instant) -> bool {
        if previous.is_some_and(|deadline| now < deadline) {
            self.quit_deadline = None;
            true
        } else {
            self.quit_deadline = Some(now + Duration::from_secs(1));
            false
        }
    }
    pub fn quit_wait(&mut self, now: Instant) -> Option<Duration> {
        let deadline = self.quit_deadline?;
        if deadline <= now {
            self.quit_deadline = None;
            None
        } else {
            Some(deadline - now)
        }
    }
    pub fn show(&mut self, prompt: Prompt) {
        self.stopping = false;
        self.quit_deadline = None;
        self.prompt = Some(prompt);
    }
}

/// Cancel stays the default: quitting with work in flight takes a choice.
pub(crate) fn sheet(app: &App) -> Option<Sheet<Action>> {
    let prompt = app.shutdown.prompt.as_ref()?;
    let (key, mut message) = match prompt {
        Prompt::Busy => ("shutdown-busy", app.i18n.text("shutdown-busy")),
        Prompt::Failed(_) => ("shutdown-failed", app.i18n.text("shutdown-failed")),
    };
    if let Prompt::Failed(error) = prompt {
        message.push_str("\n\n");
        message.push_str(&error.chars().take(256).collect::<String>());
    }
    let mut sheet = Sheet::new(key, app.i18n.text("command-quit"))
        .text("message", &message, Tone::Normal)
        .button(
            "cancel",
            app.i18n.text("shutdown-cancel"),
            Role::Normal,
            Action::CancelQuit,
            true,
        )
        .button(
            "detach",
            app.i18n.text("command-detach"),
            Role::Normal,
            Action::Detach,
            true,
        );
    if matches!(prompt, Prompt::Busy) {
        sheet = sheet.button(
            "force",
            app.i18n.text("shutdown-force"),
            Role::Destructive,
            Action::ConfirmQuit,
            app.enabled(&Action::ConfirmQuit),
        );
    }
    Some(sheet.focus("cancel"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        i18n::{I18n, Locale, LocalePreference},
        navigation::Route,
    };
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend};

    /// Draws a frame and returns where `label` appears, if it does.
    fn frame(app: &mut App, width: u16, height: u16, label: &str) -> Option<(u16, u16)> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::view::draw(frame, app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height).find_map(|y| {
            // Skip the cells a wide glyph covers, so columns stay in cells.
            let (mut line, mut x) = (String::new(), 0);
            while x < width {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                x += (unicode_width::UnicodeWidthStr::width(symbol) as u16).max(1);
            }
            line.find(label).map(|byte| {
                (
                    unicode_width::UnicodeWidthStr::width(&line[..byte]) as u16,
                    y,
                )
            })
        })
    }
    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn mouse(kind: MouseEventKind, (x, y): (u16, u16)) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn ctrl_c_quits_only_after_two_unhandled_presses_and_keeps_local_cancellation_first() {
        use crossterm::event::KeyEventKind;
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
        );
        let copy = || Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(app.input(copy()).1, None);
        assert!(frame(&mut app, 80, 24, "Press Ctrl+C again").is_some());
        assert_eq!(
            app.input(Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                KeyEventKind::Repeat
            )))
            .1,
            None
        );
        assert_eq!(app.input(copy()).1, Some(Action::Quit));

        app.input(copy());
        app.shutdown.quit_deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(
            app.input(copy()).1,
            None,
            "expired shortcuts start a fresh window"
        );
        app.input(key(KeyCode::F(1)));
        assert!(app.shutdown.quit_deadline.is_none());
        frame(&mut app, 80, 24, "");
        assert_eq!(app.input(copy()).1, None);
        assert!(!app.help, "Ctrl+C dismisses the top sheet");
        assert!(app.shutdown.quit_deadline.is_none());

        app.apply(Action::Visit(Route::Session("draft".into())));
        app.focus = crate::app::Focus::Composer;
        app.drafts
            .get_mut("draft")
            .unwrap()
            .insert("recoverable draft");
        assert_eq!(app.input(copy()).1, None);
        assert_eq!(app.drafts["draft"].text(), "");
        assert!(app.shutdown.quit_deadline.is_none());
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(
            app.drafts["draft"].text(),
            "recoverable draft",
            "clearing input remains undoable"
        );
        app.drafts.get_mut("draft").unwrap().clear();
        app.chat.select(&Route::Session("draft".into()));
        app.connection = crate::app::ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "epoch".into(),
        };
        app.chat.snapshot = Some(maka_protocol::subscription::decode_session_observation_snapshot(&serde_json::json!({
            "schemaVersion":5,"session":{"sessionId":"draft","metadataRevision":1,"status":"running","createdAt":0,"isArchived":false},
            "projectionRevision":1,"rootTurn":{"sessionId":"draft","turnId":"turn","runId":"run","status":"running"},
            "goal":null,"queue":{"hostEpoch":"epoch","queueRevision":0,"steering":[],"followup":[]},"interactions":{"pending":[]}
        })).unwrap());
        assert_eq!(
            app.input(copy()).1,
            Some(Action::StopTurn(app.stop_target().unwrap()))
        );
        assert!(app.shutdown.quit_deadline.is_none());
    }

    #[test]
    fn confirmation_defaults_to_cancel_and_never_uses_hidden_or_stale_controls() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            let mut app = App::new(
                "/unused".into(),
                I18n::new(LocalePreference::Explicit(locale), locale),
            );
            let force = app.i18n.text("shutdown-force");
            app.apply(Action::Visit(Route::Session("draft".into())));
            app.drafts.get_mut("draft").unwrap().insert("unsent draft");
            app.shutdown.show(Prompt::Busy);
            let at = frame(&mut app, 80, 24, &force).expect("force quit is offered");
            app.input(mouse(MouseEventKind::Moved, at));
            assert_eq!(
                app.input(key(KeyCode::Enter)).1,
                None,
                "hover must not arm Enter"
            );
            assert!(app.shutdown.prompt.is_none());
            app.shutdown.show(Prompt::Busy);
            frame(&mut app, 80, 24, &force);
            let click = mouse(MouseEventKind::Down(MouseButton::Left), at);
            assert_eq!(app.input(click.clone()).1, Some(Action::ConfirmQuit));
            app.input(Event::Resize(24, 6));
            assert!(app.input(key(KeyCode::Enter)).1.is_none());
            assert!(frame(&mut app, 24, 6, &force).is_none());
            assert!(app.input(key(KeyCode::Enter)).1.is_none());
            assert!(app.input(click).1.is_none(), "no stale geometry");
            frame(&mut app, 80, 24, &force);
            app.input(mouse(MouseEventKind::Down(MouseButton::Left), (0, 0)));
            assert!(app.shutdown.prompt.is_none(), "outside cancels");
            assert_eq!(app.drafts["draft"].text(), "unsent draft");
            app.shutdown.show(Prompt::Failed("uncertain result".into()));
            assert!(frame(&mut app, 80, 24, &force).is_none());
            assert!(!app.enabled(&Action::ConfirmQuit));
            app.input(key(KeyCode::Tab));
            assert_eq!(app.input(key(KeyCode::Enter)).1, Some(Action::Detach));
            app.shutdown = State {
                stopping: true,
                ..Default::default()
            };
            app.closing = true;
            app.input(key(KeyCode::Esc));
            assert!(
                app.closing,
                "an admitted shutdown cannot be cancelled by hiding its UI"
            );
        }
    }
}
