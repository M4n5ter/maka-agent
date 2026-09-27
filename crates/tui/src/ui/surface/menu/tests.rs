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
use ratatui::{Terminal, backend::TestBackend};

fn item(key: &str, label: &str, action: u8, enabled: bool) -> MenuItem<u8> {
    MenuItem {
        key: key.into(),
        label: label.into(),
        action,
        enabled,
        role: Role::Normal,
    }
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn click(x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}
fn draw(
    surface: &mut Surface<u8>,
    identity: &str,
    items: Vec<MenuItem<u8>>,
    width: u16,
    height: u16,
) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let context = Context {
                colors: Palette::default(),
                ascii: false,
                focused: true,
            };
            surface.render(
                frame,
                Rect::new(0, 0, width, 1),
                Node::text("menu", vec![("Actions".into(), crate::ui::Tone::Normal)]).on(
                    On::Menu {
                        identity: identity.into(),
                        items: items.clone(),
                    },
                ),
                context,
            );
            surface.repaint_popover(frame, &context);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn menu_freezes_actions_order_and_labels_while_revalidating_its_object_and_rows() {
    let mut surface = Surface::default();
    let original = vec![
        item("rename", "Rename original", 1, true),
        item("archive", "Archive original", 2, true),
    ];
    draw(
        &mut surface,
        "root/epoch/session/a",
        original.clone(),
        40,
        10,
    );
    assert!(surface.open_menu("menu"));
    // Even another Enter before the new layer was presented cannot dispatch.
    assert_eq!(surface.input(&key(KeyCode::Enter)).message, None);
    surface.input(&key(KeyCode::Down));
    let updated = vec![
        item("archive", "Changed archive", 22, true),
        item("rename", "Changed rename", 11, true),
    ];
    let screen = draw(
        &mut surface,
        "root/epoch/session/a",
        updated.clone(),
        40,
        10,
    );
    assert!(screen.contains("Rename original") && screen.contains("Archive original"));
    assert!(!screen.contains("Changed archive"));
    assert_eq!(surface.input(&key(KeyCode::Enter)).message, Some(2));
    assert!(!surface.captures());
    draw(
        &mut surface,
        "root/epoch/session/a",
        original.clone(),
        40,
        10,
    );
    surface.open_menu("menu");
    draw(
        &mut surface,
        "root/epoch/session/b",
        original.clone(),
        40,
        10,
    );
    assert!(
        !surface.captures(),
        "same control cannot transfer an open menu to another object"
    );
    surface.open_menu("menu");
    draw(
        &mut surface,
        "root/epoch/session/b",
        vec![item("rename", "Rename", 11, false)],
        40,
        10,
    );
    assert_eq!(surface.input(&key(KeyCode::Enter)).message, None);
    surface.input(&key(KeyCode::Down));
    assert_eq!(
        surface.input(&key(KeyCode::Enter)).message,
        None,
        "removed rows cannot dispatch"
    );
    assert!(surface.captures());
    surface.input(&key(KeyCode::Esc));
    assert_eq!(surface.focused(), Some("menu"));
}

#[test]
fn narrow_one_line_owner_opens_full_frame_menu_with_keyboard_wheel_and_pointer() {
    let mut surface = Surface::default();
    let items: Vec<_> = (0..12)
        .map(|index| {
            item(
                &index.to_string(),
                &format!("Operation {index}"),
                index,
                true,
            )
        })
        .collect();
    draw(&mut surface, "object", items.clone(), 24, 8);
    surface.input(&key(KeyCode::Enter));
    let screen = draw(&mut surface, "object", items.clone(), 24, 8);
    assert!(screen.contains("Operation 0"));
    assert!(
        surface.captures(),
        "owner has only one row but menu uses the frame"
    );
    surface.input(&key(KeyCode::End));
    let screen = draw(&mut surface, "object", items.clone(), 24, 8);
    assert!(screen.contains("Operation 11"));
    let popup = surface
        .committed
        .as_ref()
        .unwrap()
        .popover
        .as_ref()
        .unwrap();
    let last = *popup.rows.last().unwrap();
    assert_eq!(surface.input(&click(last.x, last.y)).message, Some(11));
    surface.input(&key(KeyCode::Enter));
    draw(&mut surface, "object", items.clone(), 24, 8);
    surface.input(&key(KeyCode::End));
    draw(&mut surface, "object", items.clone(), 24, 8);
    let popup = surface
        .committed
        .as_ref()
        .unwrap()
        .popover
        .as_ref()
        .unwrap();
    let penultimate = popup.rows[popup.rows.len() - 2];
    surface.input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: penultimate.x,
        row: penultimate.y,
        modifiers: KeyModifiers::NONE,
    }));
    draw(&mut surface, "object", items.clone(), 24, 8);
    assert_eq!(
        surface.input(&click(penultimate.x, penultimate.y)).message,
        Some(10),
        "hover and repaint must keep the operation beneath the pointer"
    );
    surface.input(&key(KeyCode::Enter));
    draw(&mut surface, "object", items.clone(), 24, 8);
    let wheel = Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 5,
        row: 2,
        modifiers: KeyModifiers::NONE,
    });
    surface.input(&wheel);
    draw(&mut surface, "object", items.clone(), 24, 8);
    assert_eq!(surface.input(&key(KeyCode::Enter)).message, Some(1));
    surface.input(&key(KeyCode::Enter));
    draw(&mut surface, "object", items, 24, 8);
    let committed = surface.committed.as_ref().unwrap();
    let popup = committed.popover.as_ref().unwrap().rect;
    let owner = committed
        .items
        .iter()
        .find(|item| item.id == "menu")
        .unwrap()
        .rect;
    let point = (owner.x..owner.right())
        .map(|x| Position::new(x, owner.y))
        .find(|point| !popup.contains(*point))
        .expect("the owner has visible cells outside the popup");
    let outside = surface.input(&click(point.x, point.y));
    assert!(
        outside.consumed && outside.message.is_none() && !surface.captures(),
        "outside click dismisses without reopening the owner behind it"
    );
}

#[test]
fn changed_control_type_disabled_owner_and_ambiguous_keys_retire_the_menu() {
    let mut surface = Surface::default();
    let items = vec![item("same", "Original", 1, true)];
    draw(&mut surface, "object", items.clone(), 30, 10);
    surface.open_menu("menu");
    draw(
        &mut surface,
        "object",
        vec![
            item("same", "First", 1, true),
            item("same", "Second", 2, true),
        ],
        30,
        10,
    );
    assert!(!surface.captures());
    draw(&mut surface, "object", items, 30, 10);
    surface.open_menu("menu");
    let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
    terminal
        .draw(|frame| {
            surface.render(
                frame,
                frame.area(),
                Node::text("menu", vec![]).on(On::Activate(9)),
                Context {
                    colors: Palette::default(),
                    ascii: false,
                    focused: true,
                },
            )
        })
        .unwrap();
    assert!(!surface.captures());
    let items = vec![item("same", "Original", 1, true)];
    draw(&mut surface, "object", items.clone(), 30, 10);
    surface.open_menu("menu");
    terminal
        .draw(|frame| {
            surface.render(
                frame,
                frame.area(),
                Node::text("menu", vec![])
                    .on(On::Menu {
                        identity: "object".into(),
                        items: items.clone(),
                    })
                    .enabled(false),
                Context {
                    colors: Palette::default(),
                    ascii: false,
                    focused: true,
                },
            );
        })
        .unwrap();
    assert!(!surface.captures(), "disabled object retires the menu");
}
