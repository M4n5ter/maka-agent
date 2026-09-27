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

//! Local action menus keep the actions the user opened, across view updates.
use super::*;
use crate::ui::node::MenuItem;

pub(super) struct Frozen<M> {
    identity: String,
    items: Vec<MenuItem<M>>,
    presented: bool,
}

pub(super) fn open<M: Clone>(owner: String, identity: &str, items: &[MenuItem<M>]) -> Popover<M> {
    Popover {
        owner,
        highlighted: items.iter().position(|item| item.enabled).unwrap_or(0),
        menu: Some(Frozen {
            identity: identity.into(),
            items: items.to_vec(),
            presented: false,
        }),
    }
}

pub(super) fn shell_shortcut(key: KeyEvent) -> bool {
    (key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key.code,
            KeyCode::Char('q' | 'k' | 'p' | 'P' | 'w' | 'b') | KeyCode::PageUp | KeyCode::PageDown
        ))
        || (key.modifiers == KeyModifiers::ALT
            && matches!(key.code, KeyCode::Left | KeyCode::Right))
}

impl<M: Clone> Surface<M> {
    /// A caller may observe its object's retirement between paint and input.
    /// Returns whether that event must be consumed instead of passing through.
    pub fn dismiss_menu_unless(&mut self, identity: &str) -> bool {
        let stale = self
            .popover
            .as_ref()
            .and_then(|popover| popover.menu.as_ref())
            .is_some_and(|menu| menu.identity != identity);
        if stale {
            self.popover = None;
        }
        stale
    }

    /// Opens the already presented menu of a row whose Enter action selects it.
    /// No new action or geometry is synthesized by the caller.
    pub fn open_menu(&mut self, owner: &str) -> bool {
        let Some(item) = self.committed.as_ref().and_then(|committed| {
            committed
                .items
                .iter()
                .find(|item| item.id == owner && item.enabled && !item.rect.is_empty())
        }) else {
            return false;
        };
        let On::Menu { identity, items } = &item.on else {
            return false;
        };
        self.open_popover(open(owner.into(), identity, items));
        self.set_focus(owner.into());
        true
    }

    pub(super) fn draw_menu(
        &mut self,
        frame: &mut Frame<'_>,
        items: &[Item<M>],
        context: &Context,
        previous: Option<usize>,
    ) -> Option<Chooser> {
        let popover = self.popover.as_mut()?;
        let frozen = popover.menu.as_mut()?;
        let current = items
            .iter()
            .find(|item| item.id == popover.owner && item.enabled && !item.rect.is_empty());
        let Some(owner) = current.filter(|item| matches!(&item.on, On::Menu { identity, items } if *identity == frozen.identity && unique(items))) else {
            self.popover = None;
            return None;
        };
        let On::Menu { items: live, .. } = &owner.on else {
            unreachable!()
        };
        if frozen.items.is_empty() || !unique(&frozen.items) {
            self.popover = None;
            return None;
        }
        // Labels, order and actions stay frozen. A missing operation is retired;
        // a new operation must wait for the menu to be opened again.
        for row in &mut frozen.items {
            row.enabled = live
                .iter()
                .find(|item| item.key == row.key)
                .is_some_and(|item| item.enabled);
        }
        let area = frame.area();
        let labels: Vec<_> = frozen
            .items
            .iter()
            .map(|item| crate::view::safe(&item.label))
            .collect();
        let width = (labels.iter().map(|label| label.width()).max().unwrap_or(0) as u16 + 4)
            .min(area.width);
        let rows = (frozen.items.len() as u16).min(area.height.saturating_sub(2));
        if rows == 0 || width < 5 {
            self.popover = None;
            return None;
        }
        let height = rows + 2;
        let y = if owner.rect.bottom().saturating_add(height) <= area.bottom() {
            owner.rect.bottom()
        } else {
            owner
                .rect
                .y
                .saturating_sub(height)
                .clamp(area.y, area.bottom().saturating_sub(height))
        };
        let x = owner
            .rect
            .right()
            .saturating_sub(width)
            .clamp(area.x, area.right().saturating_sub(width));
        let rect = Rect::new(x, y, width, height);
        let colors = context.colors;
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Block::bordered()
                .border_type(if context.ascii {
                    BorderType::Plain
                } else {
                    BorderType::Rounded
                })
                .border_style(Style::default().fg(colors.border))
                .style(colors.base()),
            rect,
        );
        popover.highlighted = popover.highlighted.min(frozen.items.len() - 1);
        let first = choice_start(
            previous,
            popover.highlighted,
            usize::from(rows),
            frozen.items.len(),
        );
        let mut rows_out = Vec::new();
        for (row, index) in (first..frozen.items.len())
            .take(usize::from(rows))
            .enumerate()
        {
            let item = &frozen.items[index];
            let line = Rect::new(rect.x + 1, rect.y + 1 + row as u16, rect.width - 2, 1);
            let foreground = if !item.enabled {
                colors.subtle
            } else {
                match item.role {
                    Role::Destructive => colors.error,
                    Role::Caution => colors.warning,
                    Role::Primary => colors.accent,
                    Role::Normal => colors.foreground,
                }
            };
            let mut style = if index == popover.highlighted {
                colors.focused()
            } else {
                Style::default()
            }
            .fg(foreground);
            if !item.enabled {
                style = style.add_modifier(Modifier::DIM);
            }
            let buffer = frame.buffer_mut();
            buffer.set_style(line, style);
            buffer.set_stringn(
                line.x + 1,
                line.y,
                &labels[index],
                usize::from(line.width.saturating_sub(2)),
                style,
            );
            rows_out.push(line);
        }
        // A clipped list advertises both remaining directions without changing
        // the row's hit area or creating another focus stop.
        for (shown, y, symbol) in [
            (first > 0, rect.y, if context.ascii { "^" } else { "↑" }),
            (
                first + usize::from(rows) < frozen.items.len(),
                rect.bottom() - 1,
                if context.ascii { "v" } else { "↓" },
            ),
        ] {
            if shown {
                frame.buffer_mut().set_string(
                    rect.right() - 2,
                    y,
                    symbol,
                    Style::default().fg(colors.muted),
                );
            }
        }
        frozen.presented = true;
        Some(Chooser {
            rect,
            rows: rows_out,
            first,
        })
    }

    pub(super) fn choose_menu(&mut self, index: usize) -> Outcome<M> {
        let Some(popover) = &self.popover else {
            return Outcome::handled(false);
        };
        let Some(frozen) = &popover.menu else {
            return Outcome::handled(false);
        };
        if !frozen.presented {
            return Outcome::handled(false);
        }
        let Some(committed) = self
            .committed
            .as_ref()
            .filter(|frame| frame.popover.is_some())
        else {
            return Outcome::handled(false);
        };
        let Some(row) = frozen.items.get(index).filter(|row| row.enabled) else {
            return Outcome::handled(false);
        };
        let live = committed.items.iter().find(|owner| owner.id == popover.owner && owner.enabled).is_some_and(|owner| {
            matches!(&owner.on, On::Menu { identity, items } if *identity == frozen.identity && items.iter().any(|item| item.key == row.key && item.enabled))
        });
        if !live {
            return Outcome::handled(false);
        }
        let action = row.action.clone();
        self.popover = None;
        Outcome::emit(action)
    }

    pub(super) fn menu_key(&mut self, code: KeyCode) -> Outcome<M> {
        let Some(popover) = &mut self.popover else {
            return Outcome::ignored();
        };
        let Some(menu) = &popover.menu else {
            return Outcome::ignored();
        };
        let last = menu.items.len().saturating_sub(1);
        let page = self
            .committed
            .as_ref()
            .and_then(|frame| frame.popover.as_ref())
            .map_or(1, |drawn| drawn.rows.len().max(1));
        match code {
            KeyCode::Up => popover.highlighted = popover.highlighted.saturating_sub(1),
            KeyCode::Down => popover.highlighted = (popover.highlighted + 1).min(last),
            KeyCode::Home => popover.highlighted = 0,
            KeyCode::End => popover.highlighted = last,
            KeyCode::PageUp => popover.highlighted = popover.highlighted.saturating_sub(page),
            KeyCode::PageDown => popover.highlighted = (popover.highlighted + page).min(last),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let index = popover.highlighted;
                return self.choose_menu(index);
            }
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.popover = None,
            _ => return Outcome::handled(false),
        }
        Outcome::handled(true)
    }
}

fn unique<M>(items: &[MenuItem<M>]) -> bool {
    let mut keys = std::collections::HashSet::new();
    items
        .iter()
        .all(|item| !item.key.is_empty() && keys.insert(&item.key))
}

#[cfg(test)]
mod tests;
