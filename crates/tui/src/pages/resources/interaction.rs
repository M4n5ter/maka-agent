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
use crate::app::App;
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseEventKind};
use ratatui::{Frame, widgets::Paragraph};

pub(crate) fn apply(app: &mut App, command: Command) {
    let capture = matches!(command, Command::Capture);
    app.resources.action(command);
    if capture && app.resources.terminal.capture {
        app.layer.focus("terminal");
    }
}

pub(crate) fn paint(app: &mut App, frame: &mut Frame<'_>, colors: crate::theme::Palette) {
    if !app.resources.visible {
        return;
    }
    if app.resources.command_form
        && let Some(area) = app.layer.slot("command")
    {
        app.resources
            .command
            .draw(frame, area, app.layer.focused("command"), colors);
    }
    if let Some(area) = app.layer.slot("terminal") {
        app.resources.terminal.resize(area.width, area.height);
        if let Some(screen) = &app.resources.terminal.screen {
            let text = if app.resources.terminal.scroll == 0 {
                screen.screen.clone()
            } else {
                let mut lines: Vec<_> = screen
                    .scrollback
                    .lines()
                    .chain(screen.screen.lines())
                    .collect();
                let blanks =
                    usize::from(screen.size.rows()).saturating_sub(screen.screen.lines().count());
                lines.extend(std::iter::repeat_n("", blanks));
                let end = lines.len().saturating_sub(app.resources.terminal.scroll);
                let start = end.saturating_sub(usize::from(area.height));
                lines[start..end].join("\n")
            };
            frame.render_widget(
                Paragraph::new(
                    text.lines()
                        .map(crate::view::safe)
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .style(colors.base()),
                area,
            );
            if app.resources.terminal.capture
                && app.resources.terminal.scroll == 0
                && screen.cursor.visible
                && screen.cursor.x < area.width
                && screen.cursor.y < area.height
            {
                frame.set_cursor_position((area.x + screen.cursor.x, area.y + screen.cursor.y));
            }
        } else {
            frame.render_widget(
                Paragraph::new(
                    app.i18n.text(
                        app.resources
                            .terminal
                            .error
                            .unwrap_or("resources-terminal-loading"),
                    ),
                )
                .style(colors.base()),
                area,
            );
        }
    }
}
pub(crate) fn input(app: &mut App, event: &Event) -> Option<bool> {
    if !app.resources.visible {
        return None;
    }
    if let (Event::Mouse(mouse), Some(area)) = (event, app.layer.slot("terminal"))
        && matches!(mouse.kind, MouseEventKind::Down(_))
        && !area.contains((mouse.column, mouse.row).into())
    {
        app.resources.terminal.capture = false;
    }
    if let (Event::Mouse(mouse), Some(area)) = (event, app.layer.slot("terminal"))
        && app.resources.terminal.mouse(*mouse, area)
    {
        return Some(true);
    }
    if !app.layer.focused("terminal") && !matches!(event, Event::Mouse(_)) {
        app.resources.terminal.capture = false;
    }
    if app.layer.focused("terminal") && app.resources.terminal.input(event) {
        return Some(true);
    }
    if !app.resources.command_form || app.resources.busy || app.resources.mutation != Mutation::Idle
    {
        return None;
    }
    let focused = app.layer.focused("command");
    match event {
        Event::Key(key)
            if focused
                && key.kind != KeyEventKind::Release
                && !crate::shutdown::ctrl_c(event)
                && !(key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('q'))
                && !matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
                ) =>
        {
            Some(app.resources.command.key(*key))
        }
        Event::Paste(text) if focused => Some(app.resources.command.insert(text)),
        Event::Mouse(mouse) if app.resources.command.takes(mouse) => {
            if matches!(mouse.kind, MouseEventKind::Down(_)) {
                app.layer.focus("command");
            }
            Some(app.resources.command.mouse(*mouse))
        }
        _ => None,
    }
}
