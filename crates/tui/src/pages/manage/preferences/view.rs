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

use super::{Body, Command, SecretAction};
use crate::{
    app::{Action, App},
    pages::manage::{Command as Manage, Dialog},
    ui::{self, Node, On, Role, Sheet, Size, Tone},
    view::{form, safe},
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use maka_protocol::configuration::policy::ProxyProtocol;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

const ROWS: &str = "fields/rows";
const PROXY_LABELS: [&str; 7] = [
    "proxy-host",
    "proxy-port",
    "proxy-username",
    "proxy-password",
    "proxy-bypass",
    "proxy-auto-bypass",
    "proxy-test-url",
];
pub(super) fn row_path(index: usize) -> String {
    format!("{ROWS}/{index}")
}
fn row(path: &str) -> Option<usize> {
    path.strip_prefix(ROWS)?.strip_prefix('/')?.parse().ok()
}
fn action(command: Command) -> Action {
    Action::Manage(Manage::Preferences(command))
}
fn label(app: &App, body: &Body, index: usize) -> String {
    match body {
        Body::Proxy(_) => app.i18n.text(PROXY_LABELS[index]),
        Body::Provider(f) => {
            if f.fields[index].key == "baseUrl" {
                app.i18n.text("provider-base-url")
            } else {
                safe(&f.fields[index].label)
            }
        }
        Body::Overlay(_) => app.i18n.text("request-overlay-json"),
        Body::Headers(_) => app.i18n.text(if index.is_multiple_of(2) {
            "headers-name"
        } else {
            "headers-value"
        }),
        _ => String::new(),
    }
}
fn count(body: &Body) -> usize {
    match body {
        Body::Proxy(p) => p.fields.len(),
        Body::Provider(f) => f.fields.len(),
        Body::Headers(h) => h.rows.len() * 2,
        Body::Overlay(_) => 1,
        _ => 0,
    }
}
fn width(app: &App, body: &Body) -> u16 {
    form::label_width(
        (0..count(body)).map(|index| label(app, body, index).width()),
        app.frame_size.map_or(80, |(w, _)| w),
    )
}
fn slot(app: &App, index: usize) -> Node<Action> {
    Node::slot(index.to_string(), 1)
        .on(On::Activate(action(Command::Field(index))))
        .enabled(app.preferences_enabled(&Command::Field(index)))
}
fn value_row(key: &str, label: String, value: String, width: u16) -> Node<Action> {
    Node::row(
        key.to_owned(),
        vec![
            Node::text("label", vec![(label, Tone::Muted)])
                .clip()
                .size(Size::Fixed(width)),
            Node::text("value", vec![(value, Tone::Accent)])
                .clip()
                .size(Size::Fill),
        ],
    )
}
fn toggle(
    app: &App,
    key: &str,
    label: &str,
    enabled: bool,
    command: Command,
    width: u16,
) -> Node<Action> {
    value_row(
        key,
        app.i18n.text(label),
        app.i18n.text(if enabled {
            "model-profile-on"
        } else {
            "model-profile-off"
        }),
        width,
    )
    .on(On::Activate(action(command.clone())))
    .enabled(app.preferences_enabled(&command))
}
fn secret_choices(
    app: &App,
    selected: SecretAction,
    header: Option<usize>,
    width: u16,
) -> Node<Action> {
    let actions = [
        SecretAction::Keep,
        SecretAction::Replace,
        SecretAction::Delete,
    ];
    let command = |a| header.map_or(Command::Secret(a), |index| Command::HeaderSecret(index, a));
    value_row(
        &header.map_or("secret-mode".into(), |index| format!("secret-{index}")),
        app.i18n.text("preferences-secret-action"),
        app.i18n.text(selected.label()),
        width,
    )
    .on(On::Choose {
        choices: actions
            .iter()
            .map(|&a| ui::Choice {
                label: app.i18n.text(a.label()),
                action: action(command(a)),
            })
            .collect(),
        current: actions.iter().position(|a| *a == selected),
    })
    .enabled(app.preferences_enabled(&command(selected)))
}
pub(in crate::pages::manage) fn sheet(app: &App, dialog: &Dialog) -> Sheet<Action> {
    let state = dialog.preferences.as_ref().expect("preferences state");
    let busy = app.management.preferences_pending.is_some() || state.requested.is_some();
    let mut rows = vec![];
    let width = width(app, &state.body);
    match &state.body {
        Body::Proxy(p) => {
            rows.push(toggle(
                app,
                "enabled",
                "proxy-enabled",
                p.value.enabled,
                Command::Enabled,
                width,
            ));
            let protocols = [
                ProxyProtocol::Http,
                ProxyProtocol::Https,
                ProxyProtocol::Socks5,
            ];
            let protocol_label = |p| match p {
                ProxyProtocol::Http => "HTTP",
                ProxyProtocol::Https => "HTTPS",
                ProxyProtocol::Socks5 => "SOCKS5",
            };
            rows.push(
                value_row(
                    "protocol",
                    app.i18n.text("proxy-protocol"),
                    protocol_label(p.value.protocol).into(),
                    width,
                )
                .on(On::Choose {
                    choices: protocols
                        .iter()
                        .map(|&p| ui::Choice {
                            label: protocol_label(p).into(),
                            action: action(Command::Protocol(p)),
                        })
                        .collect(),
                    current: protocols.iter().position(|v| *v == p.value.protocol),
                })
                .enabled(app.preferences_enabled(&Command::Protocol(p.value.protocol))),
            );
            rows.extend([slot(app, 0), slot(app, 1)]);
            rows.push(toggle(
                app,
                "authentication",
                "proxy-authentication",
                p.value.auth_enabled,
                Command::Authentication,
                width,
            ));
            if p.value.auth_enabled {
                rows.push(slot(app, 2));
                rows.push(secret_choices(app, p.secret_action, None, width));
                if p.secret_action == SecretAction::Replace {
                    rows.push(slot(app, 3));
                }
            }
            rows.extend([slot(app, 4), slot(app, 5), slot(app, 6)]);
        }
        Body::Provider(f) => {
            for (index, field) in f.fields.iter().enumerate() {
                if field.editor.is_some() {
                    rows.push(slot(app, index));
                } else {
                    let display = |v: &serde_json::Value| match v {
                        serde_json::Value::String(v) => safe(v),
                        serde_json::Value::Bool(v) => app.i18n.text(if *v {
                            "model-profile-on"
                        } else {
                            "model-profile-off"
                        }),
                        serde_json::Value::Null => app.i18n.text("thinking-default"),
                        v => v.to_string(),
                    };
                    rows.push(
                        value_row(
                            &index.to_string(),
                            label(app, &state.body, index),
                            display(&field.value),
                            width,
                        )
                        .on(On::Choose {
                            choices: field
                                .choices
                                .iter()
                                .enumerate()
                                .map(|(choice, v)| ui::Choice {
                                    label: display(v),
                                    action: action(Command::Choice(index, choice)),
                                })
                                .collect(),
                            current: field.choices.iter().position(|v| v == &field.value),
                        })
                        .enabled(app.preferences_enabled(&Command::Choice(index, 0))),
                    );
                }
            }
        }
        Body::Headers(h) => {
            for (index, header) in h.rows.iter().enumerate() {
                if header.action == SecretAction::Delete {
                    rows.push(Node::text(
                        format!("deleted-{index}"),
                        vec![(safe(header.name.text()), Tone::Muted)],
                    ));
                } else {
                    rows.push(slot(app, index * 2));
                }
                rows.push(secret_choices(app, header.action, Some(index), width));
                if header.action == SecretAction::Replace {
                    rows.push(slot(app, index * 2 + 1));
                }
            }
        }
        Body::Overlay(o) => {
            rows.push(
                Node::slot(
                    "0",
                    o.editor
                        .rows(
                            ui::content_width(app.frame_size.map_or(80, |(w, _)| w))
                                .saturating_sub(width),
                        )
                        .clamp(2, 8),
                )
                .on(On::Activate(action(Command::Field(0))))
                .enabled(app.preferences_enabled(&Command::Field(0))),
            );
        }
        Body::LoadingProxy | Body::LoadingHeaders => {}
    }
    let invalid = match &state.body {
        Body::Proxy(p) => p.update().err(),
        Body::Provider(f) => f.value().err(),
        Body::Headers(h) => h.update().err(),
        Body::Overlay(o) => o.value().err(),
        _ => None,
    };
    let help = match &state.body {
        Body::Proxy(_) => "proxy-note",
        Body::Headers(_) => "headers-note",
        Body::Provider(_) => "connection-preferences-note",
        Body::Overlay(_) => "request-overlay-note",
        _ => "credential-loading",
    };
    let note = if busy {
        "preferences-working"
    } else if let Some(error) = dialog.error.or(invalid) {
        error
    } else if state.saved {
        "preferences-saved"
    } else {
        help
    };
    let mut content = vec![];
    if !dialog.target.name.is_empty() {
        content.push(Node::text(
            "name",
            vec![(safe(&dialog.target.name), Tone::Muted)],
        ));
    }
    let note = Node::text(
        "note",
        vec![(
            app.i18n.text(note),
            if dialog.error.or(invalid).is_some() {
                Tone::Warning
            } else {
                Tone::Subtle
            },
        )],
    );
    let help: Node<Action> = Node::text("help", vec![(app.i18n.text(help), Tone::Subtle)]);
    let content_width =
        ui::content_width(app.frame_size.map_or(80, |(width, _)| width)).saturating_sub(1);
    // Reserve only the ordinary help's wrapped height. Live validation and
    // successful typing must not move the fields or put Add under the old Save
    // position; a longer error still receives all rows it needs.
    let note_rows = note
        .required_height(content_width)
        .max(help.required_height(content_width));
    content.push(note.size(Size::Fixed(note_rows)));
    if dialog.error.is_some() && !matches!(state.body, Body::Provider(_) | Body::Overlay(_)) {
        content.push(secondary(
            app,
            "reload",
            "preferences-reload",
            Command::Reload,
        ));
    }
    if let Body::Proxy(p) = &state.body {
        content.push(Node::text(
            "credential-status",
            vec![(
                app.i18n.text(if super::basis(&p.status).is_some() {
                    "credential-configured"
                } else {
                    "credential-absent"
                }),
                Tone::Muted,
            )],
        ));
        if let Some(test) = &p.test {
            content.push(Node::text(
                "test-result",
                vec![(
                    app.i18n.format(
                        if test.ok {
                            "proxy-test-success"
                        } else {
                            "proxy-test-failed"
                        },
                        &[
                            ("latency", &test.latency_ms.to_string()),
                            (
                                "status",
                                &test
                                    .status
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "—".into()),
                            ),
                        ],
                    ),
                    if test.ok { Tone::Accent } else { Tone::Warning },
                )],
            ));
        }
    }
    content.extend(rows);
    if matches!(state.body, Body::Proxy(_)) {
        content.push(secondary(app, "test", "proxy-test", Command::Test));
    }
    if matches!(state.body, Body::Headers(_)) {
        content.push(secondary(app, "add", "headers-add", Command::AddHeader));
    }
    if matches!(state.body, Body::Overlay(_)) {
        content.push(secondary(
            app,
            "clear-overlay",
            "request-overlay-clear",
            Command::ClearOverlay,
        ));
    }
    let height = app.frame_size.map_or(24, |(_, h)| h);
    // Only the primary decision occupies the fixed footer. Secondary commands,
    // instructions and diagnostics share the fields' scroll viewport, so their
    // translated labels cannot make the entire sheet unpresentable.
    let sheet = Sheet::new(
        format!(
            "preferences:{}:{}",
            state.generation,
            dialog.error.is_some()
        ),
        app.i18n.text(dialog.kind.label(&dialog.target)),
    )
    .body(
        Node::scroll("fields", Node::column("rows", content))
            .size(Size::Upto(height.saturating_sub(10).max(3))),
    )
    .button(
        "cancel",
        app.i18n.text("session-cancel"),
        Role::Normal,
        Action::Manage(Manage::Close),
        true,
    )
    .button(
        "save",
        app.i18n.text("session-save"),
        Role::Primary,
        action(Command::Save),
        app.preferences_enabled(&Command::Save),
    );
    if dialog.error.is_some() {
        sheet.focus("cancel")
    } else {
        sheet
    }
}
fn secondary(app: &App, key: &'static str, label: &'static str, command: Command) -> Node<Action> {
    let enabled = app.preferences_enabled(&command);
    Node::text(
        key,
        vec![(
            app.i18n.text(label),
            if enabled { Tone::Accent } else { Tone::Subtle },
        )],
    )
    .on(On::Activate(action(command)))
    .enabled(enabled)
}

pub(in crate::pages::manage) fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let Some(state) = app
        .management
        .dialog
        .as_ref()
        .and_then(|d| d.preferences.as_ref())
    else {
        return;
    };
    let count = count(&state.body);
    let width = width(app, &state.body);
    let labels: Vec<_> = (0..count).map(|i| label(app, &state.body, i)).collect();
    let rects: Vec<_> = (0..count)
        .map(|i| app.layer.rect(&row_path(i)).filter(|r| !r.is_empty()))
        .collect();
    let enabled: Vec<_> = (0..count)
        .map(|i| app.preferences_enabled(&Command::Field(i)))
        .collect();
    let focused = app.layer.focused_path().and_then(row);
    let colors = app.theme.colors();
    let state = app
        .management
        .dialog
        .as_mut()
        .and_then(|d| d.preferences.as_mut())
        .unwrap();
    for index in 0..count {
        let masked = matches!(state.body, Body::Proxy(_)) && index == 3
            || matches!(state.body, Body::Headers(_)) && index % 2 == 1;
        let Some(editor) = state.editor_mut(index) else {
            continue;
        };
        if let Some(rect) = rects[index] {
            form::draw(
                frame,
                rect,
                width,
                form::Row {
                    label: &labels[index],
                    focused: enabled[index] && focused == Some(index),
                    masked,
                    placeholder: None,
                },
                editor,
                colors,
            );
        } else {
            editor.invalidate_geometry();
        }
    }
}
pub(in crate::pages::manage) fn input(
    app: &mut App,
    event: &Event,
) -> Option<(bool, Option<Action>)> {
    let focused = app
        .layer
        .focused_path()
        .and_then(row)
        .filter(|i| app.preferences_enabled(&Command::Field(*i)));
    let multiline = app
        .management
        .dialog
        .as_ref()
        .and_then(|d| d.preferences.as_ref())
        .is_some_and(|s| matches!(s.body, Body::Overlay(_)));
    let index = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let index = focused?;
            if matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab)
                || !multiline && matches!(key.code, KeyCode::Up | KeyCode::Down)
                || key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q')
            {
                return None;
            }
            if key.code == KeyCode::Enter {
                app.layer.focus_path("footer/save");
                return Some((true, None));
            }
            index
        }
        Event::Paste(_) => focused?,
        Event::Mouse(mouse) => {
            let state = app.management.dialog.as_ref()?.preferences.as_ref()?;
            (0..count(&state.body)).find(|i| {
                state.editor(*i).is_some_and(|e| e.takes(mouse))
                    && app.preferences_enabled(&Command::Field(*i))
            })?
        }
        _ => return None,
    };
    let dialog = app.management.dialog.as_mut()?;
    let state = dialog.preferences.as_mut()?;
    state.saved = false;
    if let Body::Proxy(p) = &mut state.body {
        p.test = None;
    }
    let editor = state.editor_mut(index)?;
    let changed = match event {
        Event::Key(key) => editor.key(*key),
        Event::Paste(text) => {
            if !multiline && text.chars().any(char::is_control) {
                editor.error = Some("connection-preferences-invalid");
                true
            } else {
                editor.insert(text)
            }
        }
        Event::Mouse(mouse) => {
            let changed = editor.mouse(*mouse);
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                app.layer.focus_path(&row_path(index));
            }
            changed
        }
        _ => false,
    };
    dialog.error = None;
    Some((changed, None))
}
