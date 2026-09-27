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
    app::{Action, App},
    ui::{Node, On, Role, Sheet, Size, Tone},
    view::safe,
};
use crossterm::event::{Event, KeyCode, MouseEventKind};
use ratatui::{
    Frame,
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;
fn action(command: Command) -> Action {
    Action::Attention(command)
}
/// Fixed-width buttons always sit in rows. A narrow object keeps all of its
/// actions by stacking those rows rather than clipping the last control.
fn controls(
    app: &App,
    key: &'static str,
    width: u16,
    commands: Vec<(&'static str, Command, bool)>,
) -> Node<Action> {
    let labels: Vec<_> = commands
        .iter()
        .map(|(key, command, _)| {
            app.i18n.text(if *key == "previous" {
                "extensions-back"
            } else {
                command.label()
            })
        })
        .collect();
    let needed = labels.iter().map(|label| label.width() + 4).sum::<usize>()
        + labels.len().saturating_sub(1);
    let buttons: Vec<_> = commands
        .into_iter()
        .zip(labels)
        .map(|((key, command, enabled), label)| {
            Node::button(key, label, Role::Normal)
                .on(On::Activate(action(command)))
                .enabled(enabled)
        })
        .collect();
    if needed <= usize::from(width) {
        Node::row(key, buttons).gap(1)
    } else {
        Node::column(
            key,
            buttons
                .into_iter()
                .enumerate()
                .map(|(index, button)| Node::row(index.to_string(), vec![button]))
                .collect(),
        )
    }
}
fn item(app: &App, key: &Key, title: &str, read: bool, width: u16) -> Node<Action> {
    Node::column(
        format!("{}:{}{}", key.package.len(), key.package, key.id),
        vec![
            Node::text(
                "open",
                vec![(safe(title), if read { Tone::Normal } else { Tone::Accent })],
            )
            .clip()
            .on(On::Activate(action(Command::Select(key.clone())))),
            Node::text("source", vec![(safe(&key.package), Tone::Subtle)]).clip(),
            controls(
                app,
                "actions",
                width,
                vec![
                    ("read", Command::Read(key.clone()), !read),
                    ("dismiss", Command::Dismiss(key.clone()), true),
                ],
            ),
        ],
    )
}
fn detail(app: &App, key: &Key, title: &str, width: u16) -> Sheet<Action> {
    let rows = app
        .frame_size
        .map_or(24, |(_, height)| height)
        .saturating_sub(12)
        .max(3);
    Sheet::new(
        format!("native-attention:{key:?}"),
        app.i18n.text("attention-title"),
    )
    .body(
        Node::scroll(
            "heading",
            Node::column(
                "lines",
                vec![
                    Node::text("title", vec![(safe(title), Tone::Strong)]),
                    Node::text("source", vec![(safe(&key.package), Tone::Subtle)]),
                ],
            ),
        )
        .on(On::Scroll)
        .size(Size::Upto(3)),
    )
    .field("body", None, rows, action(Command::Read(key.clone())), true)
    .body(controls(
        app,
        "actions",
        width,
        vec![
            ("copy", Command::Copy, true),
            ("dismiss", Command::Dismiss(key.clone()), true),
        ],
    ))
    .back(action(Command::Back))
    .button(
        "back",
        app.i18n.text("extensions-back"),
        Role::Normal,
        action(Command::Back),
        true,
    )
}
pub(crate) fn sheet(app: &App) -> Option<Sheet<Action>> {
    let state = &app.attention;
    if !state.visible {
        return None;
    }
    let width = crate::ui::content_width(app.frame_size.map_or(80, |(width, _)| width));
    if let Some(key) = &state.selected
        && let Some(item) = state.items.get(key)
    {
        return Some(detail(
            app,
            key,
            &item.delivery.notice.notification.title,
            width,
        ));
    }
    let mut sheet = Sheet::new("native-attention:list", app.i18n.text("attention-title"));
    if state.full {
        sheet = sheet.text("full", &app.i18n.text("attention-full"), Tone::Warning);
    }
    let mut items: Vec<_> = state.items.iter().collect();
    items.sort_by_key(|(_, item)| std::cmp::Reverse(item.order));
    let height = app
        .frame_size
        .map_or(24, |(_, height)| height)
        .saturating_sub(12)
        .max(3);
    // A bounded page can still scroll when translated actions require a second row.
    let count = usize::from(height / 4).max(1);
    if items.is_empty() {
        sheet = sheet.text("empty", &app.i18n.text("attention-empty"), Tone::Subtle);
    } else {
        let rows = items
            .iter()
            .skip(state.offset)
            .take(count)
            .map(|(key, notice)| {
                item(
                    app,
                    key,
                    &notice.delivery.notice.notification.title,
                    notice.read,
                    width,
                )
            })
            .collect();
        sheet = sheet.body(
            Node::scroll("notices", Node::column("items", rows).gap(1)).size(Size::Upto(height)),
        );
    }
    let mut paging = Vec::new();
    if state.offset > 0 {
        paging.push((
            "previous",
            Command::Page(state.offset.saturating_sub(count)),
            true,
        ));
    }
    if state.offset + count < items.len() {
        paging.push(("next", Command::Page(state.offset + count), true));
    }
    if !paging.is_empty() {
        sheet = sheet.body(controls(app, "paging", width, paging));
    }
    Some(
        sheet
            .button(
                "close",
                app.i18n.text("session-remove-close"),
                Role::Normal,
                action(Command::Close),
                true,
            )
            .focus("close"),
    )
}
pub(crate) fn paint(app: &mut App, frame: &mut Frame<'_>, colors: crate::theme::Palette) {
    if !app.attention.visible {
        return;
    }
    let Some(area) = app.layer.slot("body") else {
        return;
    };
    let Some(item) = app
        .attention
        .selected
        .as_ref()
        .and_then(|key| app.attention.items.get(key))
    else {
        return;
    };
    let text = item
        .delivery
        .notice
        .notification
        .body
        .lines()
        .map(safe)
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((app.attention.scroll, 0))
            .style(colors.base()),
        area,
    );
}
pub(crate) fn input(app: &mut App, event: &Event) -> Option<bool> {
    if !app.attention.visible || app.attention.selected.is_none() {
        return None;
    }
    let area = app.layer.slot("body")?;
    let direction = match event {
        Event::Mouse(mouse) if area.contains((mouse.column, mouse.row).into()) => {
            match mouse.kind {
                MouseEventKind::ScrollUp => -3,
                MouseEventKind::ScrollDown => 3,
                _ => return None,
            }
        }
        Event::Key(key) if app.layer.focused("body") => match key.code {
            KeyCode::Up => -1,
            KeyCode::Down => 1,
            KeyCode::PageUp => -(i32::from(area.height)),
            KeyCode::PageDown => i32::from(area.height),
            _ => return None,
        },
        _ => return None,
    };
    let max = app
        .attention
        .selected
        .as_ref()
        .and_then(|key| app.attention.items.get(key))
        .map_or(0, |item| {
            item.delivery
                .notice
                .notification
                .body
                .len()
                .min(u16::MAX as usize) as u16
        });
    app.attention.scroll =
        (i32::from(app.attention.scroll) + direction).clamp(0, i32::from(max)) as u16;
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Locale, LocalePreference,
        i18n::I18n,
        ui::{Context, Layer},
    };
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    fn render(
        app: &App,
        layer: &mut Layer<Action>,
        sheet: Sheet<Action>,
        width: u16,
        height: u16,
    ) -> bool {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut shown = false;
        terminal
            .draw(|frame| {
                shown = layer.render(
                    frame,
                    frame.area(),
                    sheet,
                    Context {
                        colors: app.theme.colors(),
                        ascii: false,
                        focused: true,
                    },
                );
            })
            .unwrap();
        shown
    }
    fn hit(layer: &Layer<Action>, wide: &str, stacked: &str) -> Rect {
        let rect = layer
            .rect(wide)
            .or_else(|| layer.rect(stacked))
            .expect("visible object control");
        let bounds = layer.bounds().unwrap();
        assert_eq!(rect.height, 1);
        assert!(rect.width > 0 && rect.x >= bounds.x + 2 && rect.right() <= bounds.right() - 2);
        rect
    }
    fn activate(layer: &mut Layer<Action>, rect: Rect) -> Option<Action> {
        layer
            .input(
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x,
                    row: rect.y,
                    modifiers: KeyModifiers::NONE,
                }),
                action(Command::Close),
                None,
            )
            .message
    }
    #[test]
    fn long_notification_titles_and_detail_actions_fit_normal_narrow_surfaces() {
        let key = Key {
            package: "registered.notification.source".repeat(6),
            id: "one".into(),
        };
        let title = "A reminder with a long descriptive title 中文通知 ".repeat(6);
        for locale in Locale::ALL {
            for width in [30, 40, 100] {
                let mut app = App::new(
                    "/unused".into(),
                    I18n::new(LocalePreference::Explicit(locale), locale),
                );
                app.frame_size = Some((width, 30));
                let content = crate::ui::content_width(width);
                let mut layer = Layer::default();
                let list = Sheet::new("attention-list", app.i18n.text("attention-title"))
                    .body(item(&app, &key, &title, false, content))
                    .button(
                        "close",
                        app.i18n.text("session-remove-close"),
                        Role::Normal,
                        action(Command::Close),
                        true,
                    );
                assert!(
                    render(&app, &mut layer, list, width, 30),
                    "list {locale:?} {width}"
                );
                let row = format!("{}:{}{}", key.package.len(), key.package, key.id);
                let read = hit(
                    &layer,
                    &format!("{row}/actions/read"),
                    &format!("{row}/actions/0/read"),
                );
                assert_eq!(
                    activate(&mut layer, read),
                    Some(action(Command::Read(key.clone())))
                );
                let dismiss = hit(
                    &layer,
                    &format!("{row}/actions/dismiss"),
                    &format!("{row}/actions/1/dismiss"),
                );
                assert_eq!(
                    activate(&mut layer, dismiss),
                    Some(action(Command::Dismiss(key.clone())))
                );
                assert!(
                    render(
                        &app,
                        &mut layer,
                        detail(&app, &key, &title, content),
                        width,
                        30
                    ),
                    "detail {locale:?} {width}"
                );
                let copy = hit(&layer, "actions/copy", "actions/0/copy");
                assert_eq!(activate(&mut layer, copy), Some(action(Command::Copy)));
                let dismiss = hit(&layer, "actions/dismiss", "actions/1/dismiss");
                assert_eq!(
                    activate(&mut layer, dismiss),
                    Some(action(Command::Dismiss(key.clone())))
                );
                assert!(app.i18n.diagnostics().is_empty());
            }
        }
    }
    #[test]
    fn narrow_sheet_still_rejects_footer_overflow_and_insufficient_height() {
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
        );
        app.frame_size = Some((30, 30));
        let mut layer = Layer::default();
        let oversized = Sheet::new("overflow", "Notifications").button(
            "close",
            "This action cannot fit in the available row".into(),
            Role::Normal,
            action(Command::Close),
            true,
        );
        assert!(!render(&app, &mut layer, oversized, 30, 30));
        let key = Key {
            package: "source".into(),
            id: "one".into(),
        };
        assert!(!render(
            &app,
            &mut layer,
            detail(&app, &key, "Title", crate::ui::content_width(30)),
            30,
            10
        ));
        assert!(layer.bounds().is_none());
    }
}
