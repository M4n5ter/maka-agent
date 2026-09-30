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
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

impl App {
    /// Runs before send/steer and the ordinary editor, after trusted sheets.
    /// A popup selection consumes its whole input event, never also a message.
    pub fn completion_input(&mut self, event: &Event) -> Option<(bool, Option<Action>)> {
        let popup = self.completion.popup.as_ref()?;
        if !self.completion_current(&popup.context, popup.explicit) {
            self.completion.close();
            return Some((true, None));
        }
        if popup.surface.captures() {
            let outcome = self.completion.popup.as_mut()?.surface.input(event);
            if outcome.consumed {
                let action = outcome
                    .message
                    .and_then(|command| self.completion_action(command));
                return Some((outcome.redraw || action.is_some(), action));
            }
        }
        if crate::shutdown::ctrl_c(event) {
            return Some((true, self.completion_action(Command::Close)));
        }
        if matches!(event, Event::Key(key) if key.kind == KeyEventKind::Press && key.code == KeyCode::F(6) && key.modifiers.is_empty())
        {
            let popup = self.completion.popup.as_mut()?;
            if popup.area.is_some() {
                popup.controls = !popup.controls;
                if popup.controls {
                    popup.surface.focus("completion/header/category".into());
                }
            }
            return Some((true, None));
        }
        if self.completion.popup.as_ref()?.controls
            && matches!(event, Event::Key(_) | Event::Paste(_))
        {
            if matches!(event, Event::Key(key) if key.code == KeyCode::Esc) {
                self.completion.popup.as_mut()?.controls = false;
                return Some((true, None));
            }
            if matches!(event, Event::Key(key) if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::CONTROL)
            {
                return None;
            }
            let outcome = self.completion.popup.as_mut()?.surface.input(event);
            let action = outcome
                .message
                .and_then(|command| self.completion_action(command));
            return Some((outcome.redraw || action.is_some(), action));
        }
        let popup = self.completion.popup.as_ref()?;
        if matches!(event, Event::Paste(_)) && !popup.explicit {
            self.completion.close();
            return None;
        }
        let command = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('q') {
                    return None;
                }
                if key.code == KeyCode::BackTab
                    || (key.code == KeyCode::Tab && key.modifiers == KeyModifiers::SHIFT)
                {
                    if key.kind != KeyEventKind::Press
                        || popup.area.is_none()
                        || self.completion.reselect.is_some()
                    {
                        return Some((false, None));
                    }
                    let popup = self.completion.popup.as_mut()?;
                    popup.surface.focus("completion/header/category".into());
                    let outcome =
                        popup
                            .surface
                            .input(&Event::Key(crossterm::event::KeyEvent::new(
                                KeyCode::Enter,
                                KeyModifiers::NONE,
                            )));
                    return Some((outcome.redraw, None));
                }
                if key.code == KeyCode::Esc {
                    if popup.preview.is_some() {
                        self.completion.popup.as_mut()?.preview = None;
                        return Some((true, None));
                    }
                    Some(Command::Close)
                } else if matches!(key.code, KeyCode::Enter | KeyCode::Tab)
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
                {
                    if key.kind != KeyEventKind::Press {
                        return Some((false, None));
                    }
                    let command = popup.selected.clone().map(|id| Command::Choose {
                        generation: popup.generation,
                        id,
                    });
                    if command.is_none() {
                        return Some((false, None));
                    }
                    command
                } else if matches!(
                    key.code,
                    KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
                ) && key.modifiers.is_empty()
                {
                    if popup.preview.is_some() {
                        let outcome = self.completion.popup.as_mut()?.surface.input(event);
                        return Some((outcome.redraw, None));
                    }
                    self.completion_move(
                        matches!(key.code, KeyCode::Down | KeyCode::PageDown),
                        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                            8
                        } else {
                            1
                        },
                    );
                    return Some((true, None));
                } else if self.completion.reselect.is_some() {
                    return Some((false, None));
                } else if popup.explicit {
                    let popup = self.completion.popup.as_mut()?;
                    if popup.query.key(*key) {
                        popup.cursor = None;
                        popup.previous.clear();
                        self.completion_query_changed();
                        return Some((true, None));
                    }
                    return Some((false, None));
                } else {
                    // Editing, cursor motion and Undo stay in the original Editor.
                    // Their post-edit refresh cancels any now-obsolete request.
                    return None;
                }
            }
            Event::Mouse(mouse) => {
                let popup = self.completion.popup.as_mut()?;
                let point = (mouse.column, mouse.row).into();
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && popup.area.is_none_or(|area| !area.contains(point))
                {
                    Some(Command::Close)
                } else if popup.explicit
                    && popup.query_area.is_some_and(|area| area.contains(point))
                {
                    popup.controls = false;
                    return Some((popup.query.mouse(*mouse), None));
                } else if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) {
                    if popup.preview.is_some() {
                        let outcome = popup.surface.input(event);
                        return Some((outcome.redraw, None));
                    }
                    self.completion_move(mouse.kind == MouseEventKind::ScrollDown, 3);
                    return Some((true, None));
                } else {
                    let outcome = popup.surface.input(event);
                    if let Some(command) = outcome.message {
                        Some(command)
                    } else {
                        return Some((outcome.redraw, None));
                    }
                }
            }
            Event::Paste(text) => {
                if self.completion.reselect.is_some() {
                    return Some((false, None));
                }
                let popup = self.completion.popup.as_mut()?;
                let redraw = popup.query.insert(text);
                if redraw {
                    popup.cursor = None;
                    popup.previous.clear();
                    self.completion_query_changed();
                }
                return Some((redraw, None));
            }
            Event::Resize(_, _) => {
                self.completion.invalidate_geometry();
                return None;
            }
            _ => return Some((false, None)),
        };
        let action = command.and_then(|command| self.completion_action(command));
        Some((true, action))
    }

    fn completion_move(&mut self, down: bool, count: usize) {
        let Some(popup) = &mut self.completion.popup else {
            return;
        };
        let old = popup
            .selected
            .as_ref()
            .and_then(|id| popup.candidates.iter().position(|row| row.id == *id))
            .unwrap_or(0);
        let index = if down {
            old.saturating_add(count)
                .min(popup.candidates.len().saturating_sub(1))
        } else {
            old.saturating_sub(count)
        };
        popup.selected = popup.candidates.get(index).map(|row| row.id.clone());
        popup.preview = None;
    }
}
