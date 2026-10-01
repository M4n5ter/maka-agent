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

use super::{Action, App, Command, Page};
use crate::{
    ui::{Node, On, Role, Sheet, Size, Tone},
    view::safe,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

fn action(command: Command) -> Action {
    Action::SessionControls(command)
}
fn field_path(key: &str) -> String {
    format!("content/rows/{key}/input")
}
fn controls(app: &App, key: &'static str, commands: Vec<(&'static str, Command)>) -> Node<Action> {
    let width = app
        .frame_size
        .map_or(60, |(width, _)| width.saturating_sub(6).min(60));
    let labels: Vec<_> = commands
        .iter()
        .map(|(_, command)| app.i18n.text(command.label()))
        .collect();
    let needed = labels.iter().map(|label| label.width() + 4).sum::<usize>()
        + labels.len().saturating_sub(1);
    let buttons: Vec<_> = commands
        .into_iter()
        .zip(labels)
        .map(|((key, command), label)| {
            let enabled = app.session_controls_offered(&command);
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
fn prose(key: &'static str, text: String, tone: Tone) -> Node<Action> {
    Node::text(key, vec![(text, tone)])
}
pub(crate) fn sheet(app: &App) -> Option<Sheet<Action>> {
    let state = &app.session_controls;
    if !state.visible {
        return None;
    }
    let target = state.target.as_ref()?;
    let busy = state.pending.is_some() || state.requested.is_some();
    let editable = !busy && state.saved.is_none();
    let mut sheet = Sheet::new(
        format!(
            "session-controls:{}:{:?}:{}",
            target.session, state.page, state.forgetting
        ),
        app.i18n.text(
            if matches!(state.saved, Some(super::Checkpoint::Legacy(_))) {
                "controls-unresolved"
            } else {
                state.page.label()
            },
        ),
    );
    let mut rows = vec![];
    if !target.name.is_empty() {
        rows.push(prose("session", safe(&target.name), Tone::Normal));
    }
    if state.forgetting {
        rows.push(prose(
            "forget-note",
            app.i18n.text("controls-forget-note"),
            Tone::Warning,
        ));
        sheet = sheet
            .button(
                "close",
                app.i18n.text("session-cancel"),
                Role::Normal,
                action(Command::Keep),
                app.session_controls_offered(&Command::Keep),
            )
            .button(
                "forget",
                app.i18n.text("plugins-confirm"),
                Role::Caution,
                action(Command::ConfirmForget),
                app.session_controls_offered(&Command::ConfirmForget),
            )
            .focus("close")
            .back(action(Command::Keep));
    } else {
        if state.saved.is_some() && !state.saving && !busy {
            rows.push(prose(
                "unknown",
                app.i18n.text("controls-unknown"),
                Tone::Warning,
            ));
            rows.push(
                prose("forget", app.i18n.text("controls-forget"), Tone::Accent)
                    .on(On::Activate(action(Command::Forget)))
                    .enabled(app.session_controls_offered(&Command::Forget)),
            );
        }
        if !matches!(state.page, Page::History) {
            for (key, _) in &state.fields {
                let label = "controls-labels";
                let enabled = editable && !(*key == "labels" && state.labels_truncated);
                rows.push(Node::column(
                    *key,
                    vec![
                        prose("label", app.i18n.text(label), Tone::Subtle),
                        Node::slot("input", if *key == "labels" { 3 } else { 1 })
                            .on(On::Activate(action(Command::Save)))
                            .enabled(enabled),
                    ],
                ));
            }
            if let Some(super::Checkpoint::Legacy(saved)) = &state.saved {
                rows.push(prose(
                    "original",
                    safe(&serde_json::to_string_pretty(saved).unwrap_or_default()),
                    Tone::Subtle,
                ));
            }
            if state.page == Page::Metadata
                && !matches!(state.saved, Some(super::Checkpoint::Legacy(_)))
            {
                let command = Command::Flag(!state.flag);
                rows.push(
                    prose(
                        "flag",
                        format!(
                            "{} {}",
                            if state.flag { "[x]" } else { "[ ]" },
                            app.i18n.text("controls-flag")
                        ),
                        Tone::Normal,
                    )
                    .on(On::Activate(action(command.clone())))
                    .enabled(app.session_controls_offered(&command)),
                );
                rows.push(prose(
                    "labels-note",
                    app.i18n.text(if state.labels_truncated {
                        "controls-labels-truncated"
                    } else {
                        "controls-labels-note"
                    }),
                    if state.labels_truncated {
                        Tone::Warning
                    } else {
                        Tone::Subtle
                    },
                ));
            }

            if state.page.confirmation() {
                let note = match state.page {
                    Page::Compact => "controls-compact-note",
                    Page::MarkRead => "controls-mark-read-note",
                    Page::Interrupt => "controls-interrupt-note",
                    Page::RetractQueue => "controls-retract-queue-note",
                    _ => unreachable!(),
                };
                rows.push(prose("purpose", app.i18n.text(note), Tone::Normal));
            }
        } else {
            if let Some(turns) = &state.turns {
                for turn in &turns.contributions {
                    let title = turn
                        .user_prompt_preview
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(|p| safe(p))
                        .unwrap_or_else(|| app.i18n.text("controls-empty-turn"));
                    let open = Command::Jump(turn.turn_id.clone(), turn.first_sequence);
                    rows.push(Node::column(
                        turn.turn_id.clone(),
                        vec![
                            prose("prompt", title, Tone::Normal)
                                .clip()
                                .on(On::Activate(action(open.clone())))
                                .enabled(app.session_controls_offered(&open)),
                            controls(
                                app,
                                "actions",
                                vec![("landmarks", Command::Landmarks(turn.turn_id.clone()))],
                            ),
                        ],
                    ));
                }
                if turns.contributions.is_empty() {
                    rows.push(prose(
                        "empty",
                        app.i18n.text("controls-no-turns"),
                        Tone::Subtle,
                    ));
                }
            }
            if let Some(landmarks) = &state.landmarks {
                for landmark in &landmarks.landmarks {
                    let command = Command::Jump(landmark.turn_id.clone(), landmark.sequence);
                    rows.push(
                        Node::text(
                            format!("landmark-{}", landmark.sequence),
                            vec![(safe(&landmark.label), Tone::Normal)],
                        )
                        .clip()
                        .on(On::Activate(action(command.clone())))
                        .enabled(app.session_controls_offered(&command)),
                    );
                }
            }
            rows.push(controls(
                app,
                "turn-pages",
                vec![
                    ("previous", Command::PreviousTurns),
                    ("next", Command::NextTurns),
                ],
            ));
        }
        if busy {
            rows.push(prose(
                "working",
                app.i18n.text("controls-working"),
                Tone::Subtle,
            ));
        }
        if let Some(note) = state.note {
            rows.push(prose("result", app.i18n.text(note), Tone::Subtle));
        }
        if let Some(error) = &state.error {
            rows.push(prose("error", safe(error), Tone::Warning));
        }
        if state.initial_read().is_some() {
            rows.push(controls(app, "reload", vec![("refresh", Command::Refresh)]));
        }

        sheet = sheet.button(
            "close",
            app.i18n.text("session-cancel"),
            Role::Normal,
            action(Command::Close),
            true,
        );
        if state.page != Page::History && state.saved.is_none() {
            sheet = sheet.button(
                "save",
                app.i18n.text(if state.page.confirmation() {
                    "plugins-confirm"
                } else {
                    "controls-save"
                }),
                if matches!(state.page, Page::Interrupt | Page::RetractQueue) {
                    Role::Destructive
                } else {
                    Role::Primary
                },
                action(Command::Save),
                app.session_controls_offered(&Command::Save),
            );
        }
        if state.page.confirmation() || state.saved.is_some() {
            sheet = sheet.focus("close");
        } else if editable
            && let Some((key, _)) = state
                .fields
                .iter()
                .find(|(key, _)| !(*key == "labels" && state.labels_truncated))
        {
            sheet = sheet.focus_node(field_path(key));
        }
    }
    let height = app
        .frame_size
        .map_or(24, |(_, height)| height)
        .saturating_sub(10)
        .max(3);
    let body = Node::scroll("content", Node::column("rows", rows).gap(1)).size(Size::Upto(height));
    // Interactive descendants own focus. A scroll action on their ancestor
    // would turn the whole form into one hit target and hide its controls.
    let prose_only = state.forgetting
        || state.page.confirmation() && state.saved.is_none() && state.initial_read().is_none();
    Some(sheet.body(if prose_only {
        body.on(On::Scroll)
    } else {
        body
    }))
}
pub(crate) fn draw_fields(frame: &mut Frame<'_>, app: &mut App) {
    let state = &mut app.session_controls;
    let editable = state.pending.is_none() && state.requested.is_none() && state.saved.is_none();
    let colors = app.theme.colors();
    for (key, editor) in &mut state.fields {
        let path = field_path(key);
        let Some(rect) = app.layer.rect(&path).filter(|rect| !rect.is_empty()) else {
            editor.invalidate_geometry();
            continue;
        };
        editor.draw(
            frame,
            rect,
            editable && app.layer.focused_path() == Some(path.as_str()),
            colors,
        );
    }
}
pub(crate) fn input(app: &mut App, event: &Event) -> Option<(bool, Option<Action>)> {
    let state = &mut app.session_controls;
    if !state.visible
        || !state.rendered
        || state.pending.is_some()
        || state.requested.is_some()
        || state.saved.is_some()
    {
        return None;
    }
    for (key, editor) in &mut state.fields {
        if *key == "labels" && state.labels_truncated {
            continue;
        }
        let path = field_path(key);
        let focused = app.layer.focused_path() == Some(path.as_str());
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release && focused => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
                ) || key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('q')
                {
                    return None;
                }
                return Some((editor.key(*key), None));
            }
            Event::Paste(text) if focused => return Some((editor.insert(text), None)),
            Event::Mouse(mouse) if editor.takes(mouse) => {
                let press = matches!(mouse.kind, MouseEventKind::Down(_));
                if press {
                    app.layer.focus_path(&path);
                }
                return Some((editor.mouse(*mouse) || press && !focused, None));
            }
            _ => {}
        }
    }
    None
}
