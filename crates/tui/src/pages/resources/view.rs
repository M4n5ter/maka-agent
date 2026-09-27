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

mod details;
use super::*;
use crate::{
    app::{Action, App},
    ui::{Node, On, Role, Sheet, Size, Tone},
    view::safe,
};
use unicode_width::UnicodeWidthStr;
fn action(command: Command) -> Action {
    Action::Resources(command)
}

pub(crate) fn sheet(app: &App) -> Option<Sheet<Action>> {
    let state = &app.resources;
    if !state.visible {
        return None;
    }
    let target = state.target.as_ref()?;
    let mut sheet = Sheet::new(
        format!(
            "resources:{}:{:?}:{}:{}",
            target.session, state.selected, state.command_form, state.confirm_stop
        ),
        app.i18n.text("resources-title"),
    );
    let context = Node::column(
        "context",
        vec![
            Node::text("workspace", vec![(safe(&target.workspace), Tone::Subtle)]),
            Node::text("policy", vec![(safe(&target.policy), Tone::Subtle)]),
        ],
    );
    if state.confirm_forget {
        return Some(
            sheet
                .body(context)
                .text(
                    "warning",
                    &app.i18n.text("resources-forget-warning"),
                    Tone::Warning,
                )
                .button(
                    "close",
                    app.i18n.text("session-cancel"),
                    Role::Normal,
                    action(Command::Close),
                    true,
                )
                .button(
                    "forget",
                    app.i18n.text("resources-forget"),
                    Role::Destructive,
                    action(Command::ConfirmForget),
                    state.enabled(&Command::ConfirmForget),
                )
                .focus("close"),
        );
    }
    if let Some(saved) = &state.saved {
        sheet = sheet
            .text(
                "original",
                &app.i18n
                    .format("resources-original", &[("session", &safe(&saved.session))]),
                Tone::Normal,
            )
            .text(
                "pending",
                &app.i18n.text(if state.mutation == Mutation::Unknown {
                    "resources-unknown"
                } else {
                    "resources-working"
                }),
                Tone::Warning,
            )
            .body(controls(
                app,
                state,
                "recovery",
                [
                    (Command::Check, Role::Primary),
                    (Command::Forget, Role::Normal),
                ],
            ));
    }
    if let Some(error) = state.error.or(state.terminal.error) {
        sheet = sheet.text("error", &app.i18n.text(error), Tone::Warning);
    }
    if state.confirm_stop {
        sheet = sheet.body(context);
        if let Some(row) = state.selected() {
            sheet = sheet.text("command", &safe(&row.result.cmd), Tone::Normal);
        }
        return Some(
            sheet
                .text(
                    "stop-note",
                    &app.i18n.text("resources-stop-confirm"),
                    Tone::Warning,
                )
                .button(
                    "cancel",
                    app.i18n.text("session-cancel"),
                    Role::Normal,
                    action(Command::CancelStop),
                    true,
                )
                .button(
                    "stop",
                    app.i18n.text("resources-stop"),
                    Role::Destructive,
                    action(Command::Stop),
                    state.enabled(&Command::Stop),
                )
                .focus("cancel"),
        );
    }
    let mut catalog = vec![
        context,
        controls(
            app,
            state,
            "create",
            [
                (Command::NewTerminal, Role::Primary),
                (Command::RunForm, Role::Normal),
            ],
        ),
    ];
    if state.command_form {
        sheet = sheet
            .body(Node::column("catalog", catalog))
            .field(
                "command",
                Some(app.i18n.text("resources-command-line")),
                3,
                action(Command::Run),
                !state.busy && state.mutation == Mutation::Idle,
            )
            .body(Node::row(
                "run",
                vec![control(app, state, Command::Run, Role::Primary)],
            ));
    } else {
        let rows: Vec<_> = state
            .items
            .iter()
            .map(|(reference, row)| {
                let label = format!(
                    "{} · {}",
                    app.i18n.text(status(row.result.status)),
                    safe(&row.result.cmd)
                );
                Node::row(
                    reference.clone(),
                    vec![
                        Node::button("select", label, Role::Normal)
                            .size(Size::Fill)
                            .on(On::Activate(action(Command::Select(reference.clone()))))
                            .current(state.selected.as_ref() == Some(reference))
                            .enabled(state.enabled(&Command::Select(reference.clone()))),
                    ],
                )
            })
            .collect();
        if rows.is_empty() {
            catalog.push(Node::text(
                "empty",
                vec![(
                    app.i18n.text(if state.busy {
                        "extensions-loading"
                    } else {
                        "resources-empty"
                    }),
                    Tone::Subtle,
                )],
            ));
        } else {
            let visible_rows = if state.selected.is_some()
                && app.frame_size.is_some_and(|(_, height)| height <= 30)
            {
                1
            } else {
                5
            };
            catalog.push(
                Node::scroll("list", Node::column("items", rows)).size(Size::Upto(visible_rows)),
            );
        }
        sheet = details::body(app, state, sheet.body(Node::column("catalog", catalog)));
    }
    Some(
        sheet
            .aside(
                "refresh",
                app.i18n.text("extensions-refresh"),
                action(Command::Refresh),
                state.enabled(&Command::Refresh),
            )
            .aside(
                "more",
                app.i18n.text("resources-more"),
                action(Command::More),
                state.enabled(&Command::More),
            )
            .button(
                "close",
                app.i18n.text("session-remove-close"),
                Role::Normal,
                action(Command::Close),
                true,
            ),
    )
}

fn control(app: &App, state: &State, command: Command, role: Role) -> Node<Action> {
    Node::button(command.label(), app.i18n.text(command.label()), role)
        .on(On::Activate(action(command.clone())))
        .enabled(state.enabled(&command))
}

/// Buttons have fixed label widths; keep them on horizontal rows and wrap whole
/// controls so short terminals retain each action's label and hit area.
fn controls<const N: usize>(
    app: &App,
    state: &State,
    key: &'static str,
    commands: [(Command, Role); N],
) -> Node<Action> {
    let available = usize::from(crate::ui::content_width(
        app.frame_size.map_or(80, |(width, _)| width),
    ));
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut used = 0;
    for (command, role) in commands {
        let width = app.i18n.text(command.label()).width() + 4;
        if !row.is_empty() && used + 1 + width > available {
            rows.push(Node::row(rows.len().to_string(), std::mem::take(&mut row)).gap(1));
            used = 0;
        }
        used += usize::from(!row.is_empty()) + width;
        row.push(control(app, state, command, role));
    }
    if !row.is_empty() {
        rows.push(Node::row(rows.len().to_string(), row).gap(1));
    }
    Node::column(key, rows)
}
