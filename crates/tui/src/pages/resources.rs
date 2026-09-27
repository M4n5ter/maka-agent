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

//! Session-owned native processes and controller-backed interactive terminals.
mod actions;
mod interaction;
mod io;
mod model;
mod state;
pub mod terminal;
mod view;
use crate::editor::Editor;
pub(crate) use interaction::{apply, input, paint};
pub use io::{Failure, Output, execute};
use maka_presentation::shell::{Ownership, ResourceUpdate};
use maka_protocol::{resource::*, subscription::ResourceObservationFrame};
use maka_runtime::shell_result::{ShellMode, ShellSnapshot, ShellStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub(crate) use view::sheet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub root: String,
    pub epoch: String,
    pub session: String,
    pub workspace: String,
    pub policy: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open(Box<Target>),
    Close,
    Refresh,
    More,
    Select(String),
    NewTerminal,
    RunForm,
    Run,
    ConfirmStop,
    Stop,
    CancelStop,
    Capture,
    Controls,
    Reconnect,
    Copy,
    Check,
    Forget,
    ConfirmForget,
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open(_) => "resources-title",
            Self::Close => "session-remove-close",
            Self::Refresh => "extensions-refresh",
            Self::More => "resources-more",
            Self::Select(_) => "resources-details",
            Self::NewTerminal => "resources-new-terminal",
            Self::RunForm => "resources-command",
            Self::Run => "resources-run",
            Self::ConfirmStop | Self::Stop => "resources-stop",
            Self::CancelStop => "session-cancel",
            Self::Capture => "resources-type",
            Self::Controls => "resources-controls",
            Self::Reconnect => "resources-reconnect",
            Self::Copy => "resources-copy",
            Self::ConfirmForget => "resources-forget",
            Self::Check => "resources-check",
            Self::Forget => "resources-forget",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Pending {
    Launch { id: String },
    Stop { resource_ref: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    root: String,
    session: String,
    operation: Pending,
}
impl Checkpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        if self.root != root {
            return Err("Resource checkpoint belongs to another Root".into());
        }
        maka_runtime::interaction::entity_id(&self.session).map_err(str::to_owned)?;
        if let Pending::Launch { id } = &self.operation {
            maka_runtime::interaction::entity_id(id).map_err(str::to_owned)?;
        }
        let value = match &self.operation {
            Pending::Launch { id } => serde_json::to_value(ResourceStartInput {
                session_id: self.session.clone(),
                launch_id: id.clone(),
                command: None,
            })
            .unwrap(),
            Pending::Stop { resource_ref } => serde_json::to_value(ResourceStopInput {
                session_id: self.session.clone(),
                resource_ref: resource_ref.clone(),
            })
            .unwrap(),
        };
        match self.operation {
            Pending::Launch { .. } => {
                decode_start_input(&value).map_err(|error| error.to_string())?;
            }
            Pending::Stop { .. } => {
                decode_stop_input(&value).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Request {
    pub target: Target,
    generation: u64,
    serial: u64,
    work: Work,
}
#[derive(Clone)]
enum Work {
    Query(ResourceQueryInput),
    Check(ResourceQueryInput),
    Start(ResourceStartInput),
    Stop(ResourceStopInput),
}
impl Request {
    pub fn needs_checkpoint(&self) -> bool {
        matches!(self.work, Work::Start(_) | Work::Stop(_))
    }
    fn saved(&self) -> Option<Checkpoint> {
        let operation = match &self.work {
            Work::Start(input) => Pending::Launch {
                id: input.launch_id.clone(),
            },
            Work::Stop(input) => Pending::Stop {
                resource_ref: input.resource_ref.clone(),
            },
            _ => return None,
        };
        Some(Checkpoint {
            root: self.target.root.clone(),
            session: self.target.session.clone(),
            operation,
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mutation {
    #[default]
    Idle,
    Saving,
    Pending,
    Unknown,
}
pub struct State {
    pub visible: bool,
    target: Option<Target>,
    generation: u64,
    serial: u64,
    items: BTreeMap<String, ResourceUpdate>,
    selected: Option<String>,
    next_page: Option<(String, String)>,
    command: Editor,
    command_form: bool,
    confirm_stop: bool,
    confirm_forget: bool,
    pending: Option<Work>,
    busy: bool,
    stale: bool,
    error: Option<&'static str>,
    saved: Option<Checkpoint>,
    mutation: Mutation,
    checking: Option<ShellSnapshot>,
    ambiguous: bool,
    pub terminal: terminal::View,
}
impl Default for State {
    fn default() -> Self {
        Self {
            visible: false,
            target: None,
            generation: 0,
            serial: 0,
            items: BTreeMap::new(),
            selected: None,
            next_page: None,
            command: Editor::bounded(32 * 1024, "resources-command-limit"),
            command_form: false,
            confirm_stop: false,
            confirm_forget: false,
            pending: None,
            busy: false,
            stale: false,
            error: None,
            saved: None,
            mutation: Mutation::Idle,
            checking: None,
            ambiguous: false,
            terminal: terminal::View::default(),
        }
    }
}

fn status(status: ShellStatus) -> &'static str {
    match status {
        ShellStatus::Starting => "resources-starting",
        ShellStatus::Running => "resources-running",
        ShellStatus::Completed => "resources-completed",
        ShellStatus::Failed => "resources-failed",
        ShellStatus::TimedOut => "resources-timed-out",
        ShellStatus::Cancelled => "resources-cancelled",
        ShellStatus::Orphaned => "resources-orphaned",
    }
}

#[cfg(test)]
mod tests;
