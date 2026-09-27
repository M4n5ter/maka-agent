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

use super::{Action, App, Command, Frozen, Mode, Outcome, Receipt, Workspace};
use crate::{
    ui::{Node, On, Role, Sheet, Size, Tone},
    view::safe,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::Frame;

fn button(
    app: &App,
    sheet: Sheet<Action>,
    key: &'static str,
    command: Command,
    role: Role,
) -> Sheet<Action> {
    sheet.button(
        key,
        app.i18n.text(command.label()),
        role,
        Action::Bundle(command.clone()),
        app.bundle_enabled(&command),
    )
}

pub(crate) fn sheet(app: &App) -> Option<Sheet<Action>> {
    let state = &app.bundle;
    if !state.visible {
        return None;
    }
    let intent = state.intent.as_ref()?;
    let title = if state.import() {
        "bundle-import"
    } else {
        "bundle-export"
    };
    if state.forgetting {
        let sheet = Sheet::new(
            format!("bundle:forget:{}", state.sequence),
            app.i18n.text("bundle-forget"),
        )
        .text(
            "note",
            &app.i18n
                .text(if matches!(state.outcome, Outcome::Unknown { .. }) {
                    "bundle-forget-unknown-note"
                } else {
                    "bundle-forget-note"
                }),
            Tone::Warning,
        )
        .back(Action::Bundle(Command::Keep));
        let sheet = button(app, sheet, "cancel", Command::Keep, Role::Normal);
        return Some(
            button(app, sheet, "forget", Command::ConfirmForget, Role::Caution).focus("cancel"),
        );
    }
    if state.choosing {
        return Some(projects(app));
    }
    let step = if matches!(state.outcome, Outcome::Complete { .. }) {
        "receipt"
    } else if matches!(state.outcome, Outcome::Unknown { .. }) {
        "unknown"
    } else if state.reviewing {
        "review"
    } else if state.pending.is_some() || state.requested.is_some() {
        "loading"
    } else {
        "draft"
    };
    let key = if state.reviewing && matches!(state.outcome, Outcome::Draft) {
        format!("bundle:{title}:{step}:{}", state.sequence)
    } else {
        format!("bundle:{title}:{step}")
    };
    let mut sheet = Sheet::new(key, app.i18n.text(title));
    let pending = state.pending.is_some() || state.requested.is_some();
    let error = state.error.or(state.path.error).or(state.workspace.error);
    let summary = match &state.outcome {
        Outcome::Unknown { write } | Outcome::Complete { write, .. } => Some(write.clone()),
        Outcome::Draft if state.reviewing => state.freeze(),
        _ => None,
    };
    if let Some(write) = summary {
        let mut lines = summary_lines(app, &write);
        if let Some(error) = error {
            lines.insert(
                0,
                Node::text("error", vec![(app.i18n.text(error), Tone::Warning)]),
            );
        }
        if pending {
            sheet = sheet.text("working", &app.i18n.text("bundle-working"), Tone::Subtle);
        }
        match &state.outcome {
            Outcome::Unknown { .. } if !pending => {
                sheet = sheet.text("unknown", &app.i18n.text("bundle-unknown"), Tone::Warning);
            }
            Outcome::Complete { receipt, .. } => {
                let message = match receipt {
                    Receipt::Exported { count, bytes } => app.i18n.format(
                        "bundle-exported",
                        &[
                            ("count", &count.to_string()),
                            ("amount", &bytes.to_string()),
                        ],
                    ),
                    Receipt::Imported {
                        receipt,
                        artifacts: Some(artifacts),
                    } => app.i18n.format(
                        "bundle-imported",
                        &[
                            ("count", &receipt.session_ids.len().to_string()),
                            ("amount", &artifacts.to_string()),
                        ],
                    ),
                    Receipt::Imported {
                        receipt,
                        artifacts: None,
                    } => app.i18n.format(
                        "bundle-recovered",
                        &[("count", &receipt.session_ids.len().to_string())],
                    ),
                };
                sheet = sheet.text("receipt", &message, Tone::Accent);
            }
            Outcome::Draft => lines.push(Node::text(
                "scope",
                vec![(
                    app.i18n.text(if state.import() {
                        "bundle-import-note"
                    } else {
                        "bundle-export-note"
                    }),
                    Tone::Subtle,
                )],
            )),
            _ => {}
        }
        if let Some(detail) = &state.detail {
            lines.push(Node::text("detail", vec![(safe(detail), Tone::Subtle)]));
        }
        let height = app.frame_size.map_or(24, |(_, height)| height);
        sheet = sheet.body(
            Node::scroll("summary", Node::column("lines", lines).gap(1))
                .on(On::Scroll)
                .size(Size::Upto(height.saturating_sub(12).max(3))),
        );
        sheet = button(app, sheet, "close", Command::Close, Role::Normal);
        if matches!(state.outcome, Outcome::Draft) {
            sheet = button(app, sheet, "edit", Command::Edit, Role::Normal);
            let command = Command::Confirm;
            sheet = sheet.button(
                "confirm",
                app.i18n.text(if state.import() {
                    "bundle-import-now"
                } else {
                    "bundle-export-now"
                }),
                Role::Primary,
                Action::Bundle(command.clone()),
                app.bundle_enabled(&command),
            );
            return Some(sheet.back(Action::Bundle(Command::Edit)).focus("close"));
        }
        if !pending {
            if matches!(
                state.outcome,
                Outcome::Unknown {
                    write: Frozen::Import { .. }
                }
            ) {
                sheet = button(app, sheet, "query", Command::Query, Role::Primary);
            }
            if matches!(
                state.outcome,
                Outcome::Complete {
                    receipt: Receipt::Imported { .. },
                    ..
                }
            ) {
                sheet = button(app, sheet, "visit", Command::Visit, Role::Primary);
            }
            sheet = sheet.body(Node::row(
                "record-actions",
                vec![
                    Node::button("forget", app.i18n.text("bundle-forget"), Role::Normal)
                        .on(On::Activate(Action::Bundle(Command::Forget)))
                        .enabled(app.bundle_enabled(&Command::Forget)),
                ],
            ));
        }
        return Some(sheet.focus("close"));
    }
    let (width, height) = app.frame_size.unwrap_or((80, 24));
    // Paths keep one editable row on short terminals; their full value is
    // inspected in the scrollable review before any write can be confirmed.
    let field_rows = if height <= 24 { 1 } else { 2 };
    let rows = state
        .path
        .rows(crate::ui::content_width(width))
        .clamp(1, field_rows);
    let mut about = vec![];
    if let Some(error) = error {
        about.push(Node::text(
            "error",
            vec![(app.i18n.text(error), Tone::Warning)],
        ));
    }
    if pending {
        about.push(Node::text(
            "working",
            vec![(app.i18n.text("bundle-working"), Tone::Subtle)],
        ));
    }
    if let Mode::Export { name, session } = &intent.mode {
        about.push(
            Node::text(
                "session",
                vec![(
                    safe(if name.is_empty() { session } else { name }),
                    Tone::Normal,
                )],
            )
            .clip(),
        );
        if let Some(preview) = &state.preview {
            about.push(Node::text(
                "count",
                vec![(
                    app.i18n.format(
                        "bundle-count",
                        &[("count", &preview.session_count.to_string())],
                    ),
                    Tone::Normal,
                )],
            ));
        }
    }
    if let Some(detail) = &state.detail {
        about.push(Node::text("detail", vec![(safe(detail), Tone::Subtle)]));
    }
    about.push(Node::text(
        "host",
        vec![(app.i18n.text("bundle-host-path-note"), Tone::Subtle)],
    ));
    let reserved = if state.import() { 20 } else { 15 };
    let extra_fields = (field_rows - 1)
        * if state.import() && state.project.is_none() {
            2
        } else {
            1
        };
    sheet = sheet.body(
        Node::scroll("about", Node::column("lines", about))
            .on(On::Scroll)
            .size(Size::Upto(
                height.saturating_sub(reserved + extra_fields).clamp(1, 5),
            )),
    );
    sheet = sheet.field(
        "path",
        Some(app.i18n.text(if state.import() {
            "bundle-source"
        } else {
            "bundle-destination"
        })),
        rows,
        Action::Bundle(Command::Review),
        state.editable(),
    );
    if state.import() {
        let choices = [Command::HostPath, Command::Projects]
            .into_iter()
            .enumerate()
            .map(|(index, command)| {
                let selected = matches!(command, Command::HostPath) == state.project.is_none();
                Node::button(
                    index.to_string(),
                    app.i18n.text(command.label()),
                    Role::Normal,
                )
                .current(selected)
                .on(On::Activate(Action::Bundle(command.clone())))
                .enabled(app.bundle_enabled(&command))
            })
            .collect();
        sheet = sheet.body(Node::row("workspace-kind", choices).gap(2));
        if let Some((id, name)) = &state.project {
            sheet = sheet.text(
                "project",
                &format!("{} · {}", safe(name), safe(id)),
                Tone::Normal,
            );
        } else {
            let rows = state
                .workspace
                .rows(crate::ui::content_width(width))
                .clamp(1, field_rows);
            sheet = sheet.field(
                "workspace",
                Some(app.i18n.text("bundle-workspace")),
                rows,
                Action::Bundle(Command::Review),
                state.editable(),
            );
        }
    }
    // The draft is retained on Close; discarding is explicit and independent of writing.
    let mut actions = vec![];
    if !state.import() {
        actions.push(
            Node::button("preview", app.i18n.text("bundle-preview"), Role::Normal)
                .on(On::Activate(Action::Bundle(Command::Preview)))
                .enabled(app.bundle_enabled(&Command::Preview)),
        );
    }
    actions.push(
        Node::button("discard", app.i18n.text("bundle-forget"), Role::Normal)
            .on(On::Activate(Action::Bundle(Command::Forget)))
            .enabled(app.bundle_enabled(&Command::Forget)),
    );
    sheet = sheet.body(Node::row("draft-actions", actions).gap(1));
    sheet = button(app, sheet, "close", Command::Close, Role::Normal);
    Some(button(app, sheet, "review", Command::Review, Role::Primary))
}

fn summary_lines(app: &App, write: &Frozen) -> Vec<Node<Action>> {
    let text = |key, label: &str, value: &str| {
        Node::column(
            key,
            vec![
                Node::text("label", vec![(app.i18n.text(label), Tone::Subtle)]),
                Node::text("value", vec![(safe(value), Tone::Normal)]),
            ],
        )
    };
    match write {
        Frozen::Export {
            session,
            destination,
            digest,
            count,
        } => vec![
            text("session-id", "bundle-session-id", session),
            text("destination", "bundle-destination", destination),
            Node::text(
                "count",
                vec![(
                    app.i18n
                        .format("bundle-count", &[("count", &count.to_string())]),
                    Tone::Normal,
                )],
            ),
            text("digest", "bundle-digest", digest),
        ],
        Frozen::Import {
            source,
            workspace,
            expected,
        } => vec![
            text("source", "bundle-source", source),
            match workspace {
                Workspace::HostPath { path } => text("workspace", "bundle-workspace", path),
                Workspace::Project { id, name } => text(
                    "workspace",
                    "bundle-workspace-project",
                    &format!("{name} · {id}"),
                ),
            },
            text(
                "resolved-workspace",
                "bundle-resolved-workspace",
                &expected.resolved_workspace.host_cwd,
            ),
            Node::text(
                "inventory",
                vec![(
                    app.i18n.format(
                        "bundle-import-preview",
                        &[
                            ("count", &expected.session_count.to_string()),
                            ("artifacts", &expected.artifact_files.to_string()),
                        ],
                    ),
                    Tone::Normal,
                )],
            ),
            text(
                "bundle-digest",
                "bundle-content-digest",
                &expected.bundle_digest,
            ),
            text(
                "binding-digest",
                "bundle-binding-digest",
                &expected.binding_digest,
            ),
        ],
    }
}

fn projects(app: &App) -> Sheet<Action> {
    let catalog = &app.bundle.projects;
    let mut sheet = Sheet::new("bundle:projects", app.i18n.text("bundle-workspace-project"))
        .back(Action::Bundle(Command::Edit));
    if catalog.items.is_empty() {
        let label = if catalog.error {
            "projects-failed"
        } else if !catalog.ready() {
            "projects-loading"
        } else {
            "projects-empty"
        };
        sheet = sheet.text("empty", &app.i18n.text(label), Tone::Subtle);
    } else {
        let rows = catalog
            .items
            .iter()
            .map(|item| {
                let command = Command::SelectProject(item.id.clone());
                let label = if item.usable() {
                    safe(&item.name)
                } else {
                    format!(
                        "{} · {}",
                        safe(&item.name),
                        app.i18n.text("project-unavailable")
                    )
                };
                Node::text(
                    item.id.clone(),
                    vec![(
                        label,
                        if item.usable() {
                            Tone::Normal
                        } else {
                            Tone::Subtle
                        },
                    )],
                )
                .on(On::Activate(Action::Bundle(command.clone())))
                .enabled(app.bundle_enabled(&command))
            })
            .collect();
        let height = app.frame_size.map_or(24, |(_, height)| height);
        sheet = sheet.body(
            Node::scroll("projects", Node::column("rows", rows).focus_group())
                .size(Size::Upto(height.saturating_sub(9).max(3))),
        );
    }
    if catalog.can_previous() || catalog.can_next() {
        sheet = sheet.body(
            Node::row(
                "paging",
                [
                    ("previous", Command::PreviousProjects),
                    ("next", Command::NextProjects),
                ]
                .into_iter()
                .map(|(key, command)| {
                    Node::button(key, app.i18n.text(command.label()), Role::Normal)
                        .on(On::Activate(Action::Bundle(command.clone())))
                        .enabled(app.bundle_enabled(&command))
                })
                .collect(),
            )
            .gap(1),
        );
    }
    sheet = sheet.button(
        "refresh",
        app.i18n.text("extensions-refresh"),
        Role::Normal,
        Action::Bundle(Command::RefreshProjects),
        app.bundle_enabled(&Command::RefreshProjects),
    );
    button(app, sheet, "back", Command::Edit, Role::Normal).focus("back")
}

pub(crate) fn draw_fields(frame: &mut Frame<'_>, app: &mut App) {
    let editable = app.bundle.editable() && app.bundle.rendered;
    let colors = app.theme.colors();
    for (key, editor) in [
        ("path", &mut app.bundle.path),
        ("workspace", &mut app.bundle.workspace),
    ] {
        if let Some(rect) = app.layer.slot(key).filter(|rect| !rect.is_empty()) {
            editor.draw(frame, rect, editable && app.layer.focused(key), colors);
        } else {
            editor.invalidate_geometry();
        }
    }
}
impl App {
    pub(crate) fn bundle_sheet_input(&mut self, event: &Event) -> Option<(bool, Option<Action>)> {
        if !self.bundle.rendered
            || !self.bundle.editable()
            || self.bundle.reviewing
            || self.bundle.choosing
            || self.bundle.forgetting
        {
            return None;
        }
        for key in ["path", "workspace"] {
            if self.layer.slot(key).is_none() {
                continue;
            }
            let focused = self.layer.focused(key);
            let editor = if key == "path" {
                &mut self.bundle.path
            } else {
                &mut self.bundle.workspace
            };
            let changed = match event {
                Event::Key(key) if key.kind != KeyEventKind::Release && focused => {
                    if matches!(
                        key.code,
                        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
                    ) || key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('q')
                    {
                        return None;
                    }
                    editor.key(*key)
                }
                Event::Paste(text) if focused => {
                    if text.chars().any(char::is_control) {
                        editor.error = Some("bundle-path-invalid");
                        return Some((true, None));
                    }
                    editor.insert(text)
                }
                Event::Mouse(mouse) if editor.takes(mouse) => {
                    let press = matches!(mouse.kind, MouseEventKind::Down(_));
                    if press {
                        self.layer.focus(key);
                    }
                    editor.mouse(*mouse) || (press && !focused)
                }
                _ => continue,
            };
            if changed {
                self.bundle.error = None;
                self.bundle.detail = None;
                self.bundle.import_preview = None;
            }
            return Some((changed, None));
        }
        None
    }
}
