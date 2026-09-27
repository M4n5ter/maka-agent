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

pub(super) fn body(app: &App, state: &State, mut sheet: Sheet<Action>) -> Sheet<Action> {
    if let Some(row) = state.selected() {
        let mut actions = Vec::new();
        if let Ownership::SourceOwned {
            owner_session_id, ..
        }
        | Ownership::SourceUnavailable {
            source_session_id: owner_session_id,
        } = &row.ownership
        {
            sheet = sheet
                .text(
                    "ownership",
                    &app.i18n.text("resources-source-owned"),
                    Tone::Subtle,
                )
                .body(Node::row(
                    "source-actions",
                    vec![
                        Node::button(
                            "source-session",
                            app.i18n.text("resources-open-source"),
                            Role::Normal,
                        )
                        .on(On::Activate(Action::Visit(
                            crate::navigation::Route::Session(owner_session_id.clone()),
                        ))),
                    ],
                ));
        }
        if state
            .target
            .as_ref()
            .is_none_or(|target| target.workspace != row.result.cwd)
        {
            sheet = sheet.text("cwd", &safe(&row.result.cwd), Tone::Subtle);
        }
        if row
            .result
            .output
            .as_ref()
            .is_some_and(|output| match output {
                maka_runtime::shell_result::ShellOutput::Pipes {
                    stdout_truncated,
                    stderr_truncated,
                    ..
                } => *stdout_truncated || *stderr_truncated,
                maka_runtime::shell_result::ShellOutput::Pty { truncated, .. } => *truncated,
            })
        {
            sheet = sheet.text(
                "truncated",
                &app.i18n.text("resources-truncated"),
                Tone::Subtle,
            );
        }
        if row.result.mode == ShellMode::Pty
            && State::running(row)
            && matches!(row.ownership, Ownership::Local)
        {
            let rows = app
                .frame_size
                .map_or(24, |(_, height)| height)
                .saturating_sub(21)
                .clamp(1, 60);
            sheet = sheet.field(
                "terminal",
                Some(app.i18n.text(if state.terminal.capture {
                    "resources-captured"
                } else if !state.terminal.ready() && state.terminal.error.is_none() {
                    "resources-terminal-loading"
                } else {
                    "resources-screen"
                })),
                rows,
                action(Command::Capture),
                true,
            );
            actions.push(controls(
                app,
                state,
                "terminal-actions",
                [
                    (
                        if state.terminal.capture {
                            Command::Controls
                        } else {
                            Command::Capture
                        },
                        Role::Normal,
                    ),
                    (Command::Reconnect, Role::Normal),
                ],
            ));
        } else if let Some(output) = &row.result.output {
            let text = match output {
                maka_runtime::shell_result::ShellOutput::Pipes {
                    stdout,
                    stderr,
                    redacted,
                    ..
                } => {
                    if *redacted {
                        app.i18n.text("resources-redacted")
                    } else {
                        format!("{stdout}\n{stderr}")
                    }
                }
                maka_runtime::shell_result::ShellOutput::Pty {
                    screen, redacted, ..
                } => {
                    if *redacted {
                        app.i18n.text("resources-redacted")
                    } else {
                        screen.clone()
                    }
                }
            };
            let lines = text
                .lines()
                .enumerate()
                .map(|(index, line)| {
                    Node::text(index.to_string(), vec![(safe(line), Tone::Normal)])
                })
                .collect();
            sheet = sheet
                .body(Node::scroll("output", Node::column("lines", lines)).size(Size::Upto(12)));
        }
        actions.push(controls(
            app,
            state,
            "resource-actions",
            [
                (Command::Copy, Role::Normal),
                (Command::ConfirmStop, Role::Destructive),
            ],
        ));
        sheet = sheet.body(Node::column("process-actions", actions));
    }
    sheet
}
