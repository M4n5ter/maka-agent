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
impl State {
    pub fn enabled(&self, command: &Command) -> bool {
        match command {
            Command::Open(_) => true,
            Command::Close | Command::Controls => self.visible,
            Command::Check => self.visible && !self.busy && self.mutation == Mutation::Unknown,
            Command::Forget => self.visible && !self.busy && self.mutation == Mutation::Unknown,
            Command::ConfirmForget => {
                self.visible
                    && !self.busy
                    && self.mutation == Mutation::Unknown
                    && self.confirm_forget
            }
            _ if !self.visible || self.mutation != Mutation::Idle => false,
            Command::Refresh => !self.busy,
            Command::RunForm => true,
            Command::More => !self.busy && self.next_page.is_some(),
            Command::NewTerminal => true,
            Command::Run => self.command_form && !self.command.text().trim().is_empty(),
            Command::Select(reference) => self.items.contains_key(reference),
            Command::ConfirmStop | Command::Stop => {
                self.selected().is_some_and(|row| {
                    Self::running(row) && matches!(row.ownership, Ownership::Local)
                }) && (!matches!(command, Command::Stop) || self.confirm_stop)
            }
            Command::CancelStop => self.confirm_stop,
            Command::Capture => self.terminal.ready(),
            Command::Reconnect => self.selected().is_some_and(|row| {
                Self::running(row)
                    && row.result.mode == ShellMode::Pty
                    && matches!(row.ownership, Ownership::Local)
            }),
            Command::Copy => {
                self.terminal.screen.is_some()
                    || self
                        .selected()
                        .and_then(|row| row.result.output.as_ref())
                        .is_some_and(|output| match output {
                            maka_runtime::shell_result::ShellOutput::Pipes { redacted, .. }
                            | maka_runtime::shell_result::ShellOutput::Pty { redacted, .. } => {
                                !redacted
                            }
                        })
            }
        }
    }
    pub fn action(&mut self, command: Command) {
        if !self.enabled(&command) {
            return;
        }
        self.error = None;
        if self.busy && self.mutation == Mutation::Idle {
            // User controls overtake a readonly catalog refresh, never a write.
            self.serial += 1;
            self.busy = false;
            self.pending = None;
        }
        match command {
            Command::Open(target) => {
                if self.target.as_ref() != Some(target.as_ref()) {
                    self.generation += 1;
                    self.items.clear();
                    self.selected = None;
                    self.terminal.clear();
                    self.busy = false;
                    self.command = Editor::bounded(32 * 1024, "resources-command-limit");
                    self.command_form = false;
                    self.next_page = None;
                    self.confirm_stop = false;
                    self.confirm_forget = false;
                }
                self.target = Some(*target);
                self.visible = true;
                self.refresh();
            }
            Command::Close => {
                self.visible = false;
                self.confirm_stop = false;
                self.confirm_forget = false;
                self.generation += 1;
                self.pending = None;
                self.busy = false;
                self.terminal.clear();
                if self.mutation == Mutation::Saving {
                    self.mutation = Mutation::Unknown;
                }
            }
            Command::Refresh => self.refresh(),
            Command::More => {
                if let (Some(target), Some((revision, cursor))) =
                    (&self.target, self.next_page.take())
                {
                    self.pending = Some(Work::Query(ResourceQueryInput::ListContinue {
                        session_id: target.session.clone(),
                        revision,
                        cursor,
                    }));
                }
            }
            Command::Select(reference) => {
                if self.selected.as_ref() != Some(&reference) {
                    self.generation += 1;
                    self.terminal.clear();
                }
                self.selected = Some(reference);
                self.command_form = false;
                self.confirm_stop = false;
            }
            Command::RunForm => {
                self.command_form = true;
                self.terminal.capture = false;
            }
            Command::NewTerminal | Command::Run => {
                let Some(target) = &self.target else {
                    return;
                };
                let command =
                    matches!(command, Command::Run).then(|| self.command.text().to_owned());
                self.pending = Some(Work::Start(ResourceStartInput {
                    session_id: target.session.clone(),
                    launch_id: uuid::Uuid::new_v4().to_string(),
                    command,
                }));
            }
            Command::ConfirmStop => {
                self.confirm_stop = true;
                self.terminal.capture = false;
            }
            Command::CancelStop => self.confirm_stop = false,
            Command::Stop => {
                let Some(target) = &self.target else {
                    return;
                };
                let Some(resource_ref) = &self.selected else {
                    return;
                };
                self.pending = Some(Work::Stop(ResourceStopInput {
                    session_id: target.session.clone(),
                    resource_ref: resource_ref.clone(),
                }));
                self.confirm_stop = false;
            }
            Command::Capture => self.terminal.capture = true,
            Command::Controls => self.terminal.capture = false,
            Command::Reconnect => {
                self.generation += 1;
                self.terminal.clear();
            }
            Command::Copy => {
                self.terminal.copy = self
                    .terminal
                    .screen
                    .as_ref()
                    .map(|screen| screen.screen.clone())
                    .or_else(|| {
                        self.selected()
                            .and_then(|row| row.result.output.as_ref())
                            .map(|output| match output {
                                maka_runtime::shell_result::ShellOutput::Pipes {
                                    stdout,
                                    stderr,
                                    ..
                                } => format!("{stdout}\n{stderr}"),
                                maka_runtime::shell_result::ShellOutput::Pty { screen, .. } => {
                                    screen.clone()
                                }
                            })
                    });
            }
            Command::Check => {
                let Some(saved) = &self.saved else {
                    return;
                };
                if self
                    .target
                    .as_ref()
                    .is_none_or(|target| target.root != saved.root)
                {
                    return;
                }
                self.checking = None;
                self.ambiguous = false;
                self.pending = Some(Work::Check(match &saved.operation {
                    Pending::Launch { .. } => ResourceQueryInput::ListStart {
                        session_id: saved.session.clone(),
                    },
                    Pending::Stop { resource_ref } => ResourceQueryInput::Get {
                        session_id: saved.session.clone(),
                        resource_ref: resource_ref.clone(),
                    },
                }));
            }
            Command::Forget => self.confirm_forget = true,
            Command::ConfirmForget => {
                self.saved = None;
                self.mutation = Mutation::Idle;
                self.confirm_forget = false;
            }
        }
    }
}
