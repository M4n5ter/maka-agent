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
use crossterm::event::{Event as Input, KeyCode, KeyEventKind, KeyModifiers};
use maka_runtime::terminal::input::{InputAction, Key, Modifier, Modifiers};
impl View {
    /// Ctrl+] leaves capture; terminal Escape, Tab and Ctrl+C retain their native meaning.
    pub fn input(&mut self, event: &Input) -> bool {
        if !self.capture {
            return false;
        }
        if let Input::Key(key) = event
            && key.kind != KeyEventKind::Release
            && key.modifiers.contains(KeyModifiers::CONTROL)
            // Crossterm decodes the legacy Ctrl+] byte (0x1d) as Ctrl+5.
            && matches!(key.code, KeyCode::Char(']' | '5'))
        {
            self.capture = false;
            return true;
        }
        if !self.ready() {
            return matches!(event, Input::Key(_) | Input::Paste(_));
        }
        let Some(screen) = &self.screen else {
            return false;
        };
        let text = match event {
            Input::Key(key) if key.kind != KeyEventKind::Release => {
                if key
                    .modifiers
                    .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
                    && matches!(key.code, KeyCode::Char('c' | 'C'))
                {
                    self.copy = Some(screen.screen.clone());
                    return true;
                }
                let name = match key.code {
                    KeyCode::Char(ch)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.write(InputAction::Text(ch.to_string()));
                        return true;
                    }
                    KeyCode::Char(ch) if ch.is_ascii() => ch.to_string(),
                    KeyCode::Enter => "enter".into(),
                    KeyCode::Tab => "tab".into(),
                    KeyCode::BackTab => "tab".into(),
                    KeyCode::Esc => "escape".into(),
                    KeyCode::Backspace => "backspace".into(),
                    KeyCode::Delete => "delete".into(),
                    KeyCode::Insert => "insert".into(),
                    KeyCode::Up => "arrow_up".into(),
                    KeyCode::Down => "arrow_down".into(),
                    KeyCode::Left => "arrow_left".into(),
                    KeyCode::Right => "arrow_right".into(),
                    KeyCode::Home => "home".into(),
                    KeyCode::End => "end".into(),
                    KeyCode::PageUp => "page_up".into(),
                    KeyCode::PageDown => "page_down".into(),
                    KeyCode::F(number) => format!("f{number}"),
                    _ => return true,
                };
                let mut modifiers = vec![];
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    modifiers.push(Modifier::Ctrl);
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    modifiers.push(Modifier::Alt);
                }
                if key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab {
                    modifiers.push(Modifier::Shift);
                }
                let Ok(key) = Key::parse(&name) else {
                    return true;
                };
                let Ok(modifiers) = Modifiers::try_from(modifiers) else {
                    return true;
                };
                Some(InputAction::Key { key, modifiers })
            }
            Input::Paste(text) => {
                // Drop embedded terminal controls, including a forged paste terminator.
                let text: String = text
                    .chars()
                    .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
                    .collect();
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                (!text.is_empty()).then_some(InputAction::Paste(text))
            }
            _ => return false,
        };
        if let Some(text) = text {
            self.write(text);
        }
        true
    }
}

impl View {
    pub fn mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        area: ratatui::layout::Rect,
    ) -> bool {
        use crossterm::event::{MouseButton as Button, MouseEventKind as Kind};
        use maka_runtime::terminal::input::{
            MouseAction, MouseButton, MouseEvent, ScrollDirection,
        };
        if !area.contains((mouse.column, mouse.row).into()) {
            return false;
        }
        let Some(screen) = &self.screen else {
            return false;
        };
        if mouse.modifiers.contains(KeyModifiers::SHIFT) || !self.capture {
            match mouse.kind {
                Kind::ScrollUp => {
                    self.scroll = (self.scroll + 3).min(screen.scrollback.lines().count());
                    return true;
                }
                Kind::ScrollDown => {
                    self.scroll = self.scroll.saturating_sub(3);
                    return true;
                }
                _ => return false,
            }
        }
        if !self.ready() {
            return true;
        }
        let button = |button| match button {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
        };
        let event = match mouse.kind {
            Kind::Down(value) => MouseEvent::Press(button(value)),
            Kind::Up(value) => MouseEvent::Release(button(value)),
            Kind::Drag(value) => MouseEvent::Move(Some(button(value))),
            Kind::Moved => MouseEvent::Move(None),
            Kind::ScrollUp => MouseEvent::Scroll(ScrollDirection::Up),
            Kind::ScrollDown => MouseEvent::Scroll(ScrollDirection::Down),
            _ => return false,
        };
        let mut modifiers = vec![];
        if mouse.modifiers.contains(KeyModifiers::CONTROL) {
            modifiers.push(Modifier::Ctrl);
        }
        if mouse.modifiers.contains(KeyModifiers::ALT) {
            modifiers.push(Modifier::Alt);
        }
        let Ok(modifiers) = Modifiers::try_from(modifiers) else {
            return true;
        };
        self.write(InputAction::Mouse(MouseAction {
            x: u64::from(mouse.column - area.x),
            y: u64::from(mouse.row - area.y),
            event,
            modifiers,
        }));
        true
    }
}
