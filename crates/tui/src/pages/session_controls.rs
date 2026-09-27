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

pub mod anchor;
mod view;
mod work;
use crate::{
    app::{Action, App, ConnectionState},
    editor::Editor,
    navigation::Route,
};
use maka_client::RequestFailure;
use maka_protocol::{
    configuration::policy::{
        EnabledPolicy, Personalization, RuntimePolicyMutation, RuntimePolicyMutationInput,
        RuntimePolicySnapshot,
    },
    session::*,
};
use maka_runtime::execution::WorkspaceTarget;
use serde::{Deserialize, Serialize};
pub(crate) use view::{draw_fields, input, sheet};
pub use work::{Checkpoint, Output, Request, execute};
use work::{Mutation, Work};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    #[default]
    Preferences,
    Metadata,
    Executor,
    NewExecutor,
    Compact,
    MarkRead,
    Interrupt,
    RetractQueue,
    History,
}
impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preferences => "controls-preferences",
            Self::Metadata => "controls-metadata",
            Self::Executor => "controls-executor",
            Self::NewExecutor => "controls-new-executor",
            Self::Compact => "controls-compact",
            Self::MarkRead => "controls-mark-read",
            Self::Interrupt => "controls-interrupt",
            Self::RetractQueue => "controls-retract-queue",
            Self::History => "controls-history",
        }
    }
    fn confirmation(self) -> bool {
        matches!(
            self,
            Self::Compact | Self::MarkRead | Self::Interrupt | Self::RetractQueue
        )
    }
    fn executor(self) -> bool {
        matches!(self, Self::Executor | Self::NewExecutor)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    root: String,
    epoch: String,
    session: String,
    name: String,
    revision: u64,
    read_message: Option<String>,
    run: Option<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceTarget>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open(Target, Page),
    Reopen,
    Close,
    Refresh,
    Save,
    Workspace(bool),
    Flag(bool),
    Executor(String),
    Thinking(Option<ThinkingLevel>),
    SearchExecutors,
    NextExecutors,
    PreviousExecutors,
    NextTurns,
    PreviousTurns,
    Landmarks(String),
    Jump(String, u64),
    Forget,
    Keep,
    ConfirmForget,
    OpenCreated,
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Open(_, page) => page.label(),
            Self::Reopen => "controls-unresolved",
            Self::Close | Self::Keep => "session-cancel",
            Self::Refresh => "extensions-refresh",
            Self::Save => "controls-save",
            Self::Workspace(_) => "controls-workspace-instructions",
            Self::Flag(_) => "controls-flag",
            Self::Executor(_) => "controls-executor",
            Self::Thinking(_) => "session-thinking",
            Self::SearchExecutors => "controls-search-executors",
            Self::NextTurns | Self::NextExecutors => "sessions-next",
            Self::PreviousTurns | Self::PreviousExecutors => "sessions-previous",
            Self::Landmarks(_) => "controls-landmarks",
            Self::Jump(_, _) => "controls-open-turn",
            Self::Forget => "controls-forget",
            Self::ConfirmForget => "controls-confirm-forget",
            Self::OpenCreated => "controls-open-created",
        }
    }
}
#[derive(Default)]
pub struct State {
    anchor: anchor::State,
    pub visible: bool,
    rendered: bool,
    target: Option<Target>,
    page: Page,
    fields: Vec<(&'static str, Editor)>,
    flag: bool,
    labels_truncated: bool,
    policy: Option<RuntimePolicySnapshot>,
    executors: Option<maka_client::controls::ExecutorChoices>,
    executor: Option<String>,
    executor_cursor: Option<maka_client::controls::ExecutorCursor>,
    executor_previous: Vec<Option<maka_client::controls::ExecutorCursor>>,
    thinking: Option<ThinkingLevel>,
    turns: Option<maka_protocol::navigation::TurnsResult>,
    landmarks: Option<maka_protocol::navigation::LandmarksResult>,
    positions: Vec<u64>,
    position: u64,
    sequence: u64,
    pending: Option<Request>,
    requested: Option<Work>,
    saved: Option<Checkpoint>,
    saving: bool,
    forgetting: bool,
    error: Option<String>,
    note: Option<&'static str>,
    origin: Option<Route>,
    created: Option<String>,
}
impl State {
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        self.saved.clone()
    }
    pub fn restore(&mut self, saved: Checkpoint) {
        self.target = Some(saved.target.clone());
        self.page = saved.page;
        self.saved = Some(saved);
    }
    pub fn disconnect(&mut self) {
        self.anchor = anchor::State::default();
        self.pending = None;
        self.requested = None;
        self.saving = false;
        self.rendered = false;
        if self.saved.is_some() {
            self.note = Some("controls-unknown");
        }
    }
    pub fn invalidate_geometry(&mut self) {
        self.rendered = false;
        for (_, field) in &mut self.fields {
            field.invalidate_geometry();
        }
    }
    pub(crate) fn presented(&mut self, rendered: bool) {
        self.rendered = rendered;
    }
    fn text(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|(name, _)| *name == key)
            .map_or("", |(_, e)| e.text())
    }
    fn field(&mut self, key: &'static str, value: &str, bytes: usize) {
        let mut editor = Editor::bounded(bytes, "controls-field-limit");
        editor.insert(value);
        editor.clear_history();
        self.fields.push((key, editor));
    }
    fn load_session(&mut self, item: &SessionCatalogProjection) {
        self.fields.clear();
        self.flag = item.is_flagged;
        self.labels_truncated = item.labels_truncated;
        if self.page == Page::Metadata {
            self.field(
                "labels",
                &item
                    .labels
                    .iter()
                    .filter(|s| s.as_str() != "mode:bot")
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n"),
                16 * 1024,
            );
        }
        if self.page == Page::Executor {
            self.field("query", "", 512);
            self.field(
                "model",
                item.executor_settings
                    .as_ref()
                    .and_then(|s| s.model.as_deref())
                    .unwrap_or(""),
                512,
            );
            self.executor = item.executor_id.as_ref().map(|id| id.as_str().to_owned());
            self.thinking = item
                .executor_settings
                .as_ref()
                .and_then(|s| s.thinking_level);
        }
        if let Some(target) = &mut self.target {
            target.revision = item.revision;
            target.name = item.name.clone();
        }
    }
    fn initial_read(&self) -> Option<Work> {
        match self.page {
            Page::Preferences => Some(Work::Policy),
            Page::Metadata => Some(Work::Session),
            Page::NewExecutor if self.saved.is_some() || self.created.is_some() => {
                Some(Work::CreationStatus)
            }
            Page::Executor | Page::NewExecutor => Some(Work::Executors {
                query: self.text("query").into(),
                cursor: None,
            }),
            Page::History => Some(Work::Turns {
                position: self.position,
                through: self.turns.as_ref().and_then(|p| p.through_sequence),
            }),
            Page::Compact => self.saved.as_ref().and_then(|saved| match &saved.mutation {
                Mutation::Compact(input) => Some(Work::CompactStatus(input.clone())),
                _ => None,
            }),
            _ => None,
        }
    }
}
impl App {
    fn controls_target(&self, page: Page) -> Option<Target> {
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        if matches!(page, Page::Preferences | Page::NewExecutor) {
            return Some(Target {
                root: root_id.clone(),
                epoch: epoch.clone(),
                session: String::new(),
                name: String::new(),
                revision: 0,
                read_message: None,
                run: None,
                workspace: if page == Page::NewExecutor {
                    Some(WorkspaceTarget::HostPath {
                        path: std::env::current_dir().ok()?.to_str()?.to_owned(),
                    })
                } else {
                    None
                },
            });
        }
        let Route::Session(id) = self.navigation.current() else {
            return None;
        };
        let super::sessions::Detail::Ready(item) = &self.sessions.detail else {
            return None;
        };
        if item.id != id || self.chat.removed {
            return None;
        }
        if page != Page::History && self.session_is_managed(&id) {
            return None;
        }
        let run = self.stop_target().map(|run| (run.turn, run.run));
        let read_message = (self.chat.session.as_ref() == Some(&id))
            .then(|| self.chat.read_marker_message())
            .flatten();
        if page == Page::MarkRead && read_message.is_none()
            || page == Page::Interrupt && run.is_none()
            || page == Page::RetractQueue && self.queue_rows().is_empty()
        {
            return None;
        }
        Some(Target {
            root: root_id.clone(),
            epoch: epoch.clone(),
            session: id,
            name: item.name.clone(),
            revision: item.revision,
            read_message,
            run,
            workspace: None,
        })
    }
    pub fn session_control_commands(&self) -> Vec<(Action, &'static str)> {
        if self.session_controls.saved.is_some() {
            return vec![(
                Action::SessionControls(Command::Reopen),
                "controls-unresolved",
            )];
        }
        [
            Page::Metadata,
            Page::Executor,
            Page::Compact,
            Page::MarkRead,
            Page::Interrupt,
            Page::RetractQueue,
            Page::History,
        ]
        .into_iter()
        .filter_map(|page| {
            self.controls_target(page).map(|target| {
                (
                    Action::SessionControls(Command::Open(target, page)),
                    page.label(),
                )
            })
        })
        .collect()
    }
    pub fn personal_preferences_action(&self) -> Option<Action> {
        self.controls_target(Page::Preferences)
            .map(|target| Action::SessionControls(Command::Open(target, Page::Preferences)))
    }
    pub fn new_executor_session_action(&self) -> Option<Action> {
        if self.session_controls.saved.is_some() {
            return Some(Action::SessionControls(Command::Reopen));
        }
        self.controls_target(Page::NewExecutor)
            .map(|target| Action::SessionControls(Command::Open(target, Page::NewExecutor)))
    }
    fn project_executor_target(&self, id: &str) -> Option<Target> {
        if self.navigation.current() != Route::Projects || !self.projects.ready() {
            return None;
        }
        let project = self
            .projects
            .items
            .iter()
            .find(|item| item.id == id && item.usable())?;
        let mut target = self.controls_target(Page::Preferences)?;
        target.name = project.name.clone();
        target.workspace = Some(WorkspaceTarget::Project {
            project_id: project.id.clone(),
        });
        Some(target)
    }
    pub fn new_project_executor_session_action(&self, id: &str) -> Option<Action> {
        self.project_executor_target(id)
            .map(|target| Action::SessionControls(Command::Open(target, Page::NewExecutor)))
    }
    pub fn session_controls_enabled(&self, command: &Command) -> bool {
        self.session_controls_offered(command)
            && (matches!(
                command,
                Command::Open(_, _) | Command::Reopen | Command::Close
            ) || self.session_controls.rendered)
    }
    pub(super) fn session_controls_offered(&self, command: &Command) -> bool {
        let state = &self.session_controls;
        let connected = state.target.as_ref().is_some_and(|t| matches!(&self.connection,
            ConnectionState::Connected { root_id, epoch } if root_id == &t.root && epoch == &t.epoch));
        let ready =
            state.visible && connected && state.pending.is_none() && state.requested.is_none();
        match command {
            Command::Open(target, page) => !state.visible && state.saved.is_none() && state.pending.is_none()
                && (*page != Page::NewExecutor || self.enabled(&Action::CreateSession))
                && (if *page == Page::NewExecutor && let Some(WorkspaceTarget::Project { project_id }) = &target.workspace {
                    self.project_executor_target(project_id)
                } else { self.controls_target(*page) }).as_ref() == Some(target),
            Command::Reopen => !state.visible && state.saved.as_ref().is_some_and(|saved| matches!(&self.connection, ConnectionState::Connected { root_id, .. } if root_id == &saved.target.root)),
            Command::Close => state.visible,
            Command::Forget => ready && state.saved.is_some() && !state.forgetting,
            Command::Keep | Command::ConfirmForget => ready && state.forgetting,
            Command::Refresh => ready && !state.forgetting && state.initial_read().is_some(),
            Command::Save => ready && state.saved.is_none() && state.created.is_none() && !state.forgetting && (!state.page.confirmation() || state.note.is_none()) && match state.page {
                Page::Preferences => state.policy.is_some(),
                Page::Executor | Page::NewExecutor => (state.page != Page::NewExecutor || self.enabled(&Action::CreateSession))
                    && state.executor.as_ref().is_some_and(|id| state.executors.as_ref().is_some_and(|v| v.executors.iter().any(|e| &e.id == id))),
                Page::History => false, _ => true,
            },
            Command::Workspace(_) => ready && state.saved.is_none() && state.page == Page::Preferences && state.policy.is_some(),
            Command::Flag(_) => ready && state.saved.is_none() && state.page == Page::Metadata,
            Command::Executor(id) => ready && state.saved.is_none() && state.created.is_none() && state.page.executor() && state.executors.as_ref().is_some_and(|v| v.executors.iter().any(|e| &e.id == id)),
            Command::Thinking(_) => ready && state.saved.is_none() && state.created.is_none() && state.page.executor(),
            Command::SearchExecutors => ready && state.page.executor() && (state.page != Page::NewExecutor || state.saved.is_none() && state.created.is_none()),
            Command::NextExecutors => ready && state.page.executor() && state.created.is_none() && state.saved.is_none() && state.executors.as_ref().and_then(|page| page.next_cursor.as_ref()).is_some(),
            Command::PreviousExecutors => ready && state.page.executor() && state.created.is_none() && state.saved.is_none() && !state.executor_previous.is_empty(),
            Command::OpenCreated => ready && state.created.is_some(),
            Command::NextTurns => ready && state.turns.as_ref().and_then(|v| v.next_position).is_some(),
            Command::PreviousTurns => ready && !state.positions.is_empty(),
            Command::Landmarks(turn) => ready && state.turns.as_ref().is_some_and(|v| v.contributions.iter().any(|c| &c.turn_id == turn)),
            Command::Jump(turn, sequence) => ready && state.turns.as_ref().is_some_and(|v| v.contributions.iter().any(|c| &c.turn_id == turn && c.first_sequence == *sequence))
                || ready && state.landmarks.as_ref().is_some_and(|v| v.landmarks.iter().any(|c| &c.turn_id == turn && c.sequence == *sequence)),
        }
    }
    pub fn session_controls_action(&mut self, command: Command) -> Option<Action> {
        if !self.session_controls_enabled(&command) {
            return None;
        }
        if let Command::Jump(turn, sequence) = command {
            let session = self.session_controls.target.as_ref()?.session.clone();
            self.session_controls.visible = false;
            return self.open_session_anchor(session, Some(turn), None, sequence);
        }
        if command == Command::OpenCreated {
            let session = self.session_controls.created.clone()?;
            self.session_controls.visible = false;
            return self.apply(Action::Visit(Route::Session(session)));
        }
        let state = &mut self.session_controls;
        match command {
            Command::Open(mut target, page) => {
                if page == Page::NewExecutor {
                    target.session = uuid::Uuid::new_v4().to_string();
                }
                let sequence = state.sequence;
                *state = State {
                    target: Some(target),
                    page,
                    visible: true,
                    sequence,
                    origin: Some(self.navigation.current()),
                    ..State::default()
                };
                if page == Page::NewExecutor {
                    state.field("name", "", 1024);
                    state.field("query", "", 512);
                    state.field("model", "", 512);
                } else if let super::sessions::Detail::Ready(item) = &self.sessions.detail
                    && page != Page::Preferences
                {
                    state.load_session(item);
                }
                state.requested = state.initial_read();
            }
            Command::Reopen => {
                let saved = state.saved.as_ref()?;
                state.target = Some(saved.target.clone());
                state.page = saved.page;
                // Reads may use the new connection; the original mutation epoch stays frozen in the checkpoint.
                if let ConnectionState::Connected { epoch, .. } = &self.connection {
                    state.target.as_mut()?.epoch = epoch.clone();
                }
                state.visible = true;
                state.rendered = false;
                state.note = Some("controls-unknown");
            }
            Command::Close => {
                state.visible = false;
                state.rendered = false;
            }
            Command::Refresh => {
                state.error = None;
                if state.page.executor() {
                    state.executor_previous.clear();
                }
                state.requested = state.initial_read();
            }
            Command::Flag(flag) => state.flag = flag,
            Command::Executor(id) => {
                if state.executor.as_ref() != Some(&id) {
                    if let Some((_, model)) =
                        state.fields.iter_mut().find(|(key, _)| *key == "model")
                    {
                        *model = Editor::bounded(512, "controls-field-limit");
                    }
                    state.thinking = None;
                }
                state.executor = Some(id);
            }
            Command::Thinking(level) => state.thinking = level,
            Command::SearchExecutors => {
                state.executor_previous.clear();
                state.requested = Some(Work::Executors {
                    query: state.text("query").into(),
                    cursor: None,
                });
            }
            Command::NextExecutors => {
                let cursor = state.executors.as_ref()?.next_cursor.clone()?;
                state.executor_previous.push(state.executor_cursor.clone());
                state.requested = Some(Work::Executors {
                    query: cursor.query.clone(),
                    cursor: Some(cursor),
                });
            }
            Command::PreviousExecutors => {
                let query = state.executor_cursor.as_ref()?.query.clone();
                let cursor = state.executor_previous.pop()?;
                state.requested = Some(Work::Executors { query, cursor });
            }
            Command::Workspace(enabled) => {
                let policy = state.policy.as_ref()?;
                let mutation = Mutation::Policy(RuntimePolicyMutationInput {
                    expected_revision: policy.revision,
                    operation: RuntimePolicyMutation::SetWorkspaceInstructions {
                        value: EnabledPolicy { enabled },
                    },
                });
                state.queue_mutation(mutation);
            }
            Command::Save => match state.mutation() {
                Ok(mutation) => state.queue_mutation(mutation),
                Err(error) => state.error = Some(error),
            },
            Command::NextTurns => {
                let next = state.turns.as_ref()?.next_position?;
                state.positions.push(state.position);
                state.position = next;
                state.landmarks = None;
                state.requested = state.initial_read();
            }
            Command::PreviousTurns => {
                state.position = state.positions.pop()?;
                state.landmarks = None;
                state.requested = state.initial_read();
            }
            Command::Landmarks(turn) => state.requested = Some(Work::Landmarks(turn)),
            Command::Forget => state.forgetting = true,
            Command::Keep => state.forgetting = false,
            Command::ConfirmForget => {
                state.saved = None;
                state.forgetting = false;
                state.visible = false;
                state.note = None;
            }
            Command::Jump(_, _) | Command::OpenCreated => unreachable!(),
        }
        None
    }
    pub fn session_controls_request(&mut self) -> Option<Request> {
        let state = &mut self.session_controls;
        let target = state.target.as_ref()?;
        if self.closing
            || state.pending.is_some()
            || !matches!(&self.connection,
            ConnectionState::Connected { root_id, epoch } if root_id == &target.root && epoch == &target.epoch)
        {
            return None;
        }
        let work = state.requested.take()?;
        state.sequence += 1;
        let request = Request {
            target: target.clone(),
            work,
            sequence: state.sequence,
        };
        state.saving = request.needs_checkpoint();
        state.pending = Some(request.clone());
        Some(request)
    }
    pub fn session_controls_after_checkpoint(
        &mut self,
        request: &Request,
        result: &Result<(), String>,
    ) -> bool {
        let state = &mut self.session_controls;
        if !state.saving || !state.pending.as_ref().is_some_and(|p| p.same(request)) {
            return false;
        }
        state.saving = false;
        if result.is_ok()
            && !self.closing
            && matches!(&self.connection, ConnectionState::Connected { root_id, epoch } if root_id == &request.target.root && epoch == &request.target.epoch)
        {
            return true;
        }
        state.pending = None;
        state.saved = None;
        state.error = Some(self.i18n.text("controls-checkpoint-failed"));
        false
    }
    pub fn session_controls_completed(
        &mut self,
        request: Request,
        result: Result<Output, RequestFailure>,
    ) {
        let mut visit = None;
        let state = &mut self.session_controls;
        if !state.pending.as_ref().is_some_and(|p| p.same(&request)) {
            return;
        }
        state.pending = None;
        if !matches!(&self.connection, ConnectionState::Connected { root_id, epoch } if root_id == &request.target.root && epoch == &request.target.epoch)
        {
            return;
        }
        match result {
            Ok(output) => {
                if request.needs_checkpoint() {
                    state.saved = None;
                    state.note = Some("controls-saved");
                }
                state.error = None;
                match output {
                    Output::Created(item) => {
                        state.created = Some(item.id.clone());
                        if state.visible && state.origin.as_ref() == Some(&self.navigation.current()) {
                            visit = Some(item.id.clone());
                            state.visible = false;
                        }
                        self.sessions.updated(item);
                        self.inbox.refresh();
                    }
                    Output::CreationStatus(item) => {
                        // A current object does not prove which request created it.
                        state.created = item.as_ref().map(|item| item.id.clone());
                        state.note = Some(if item.is_some() { "controls-create-found" } else { "controls-create-missing" });
                        if let Some(item) = item { self.sessions.updated(item); }
                    }
                    Output::Policy(snapshot) => {
                        // Updating the independent workspace switch must not erase a
                        // personalization draft that has not been submitted yet.
                        if state.fields.is_empty() {
                            state.field("display-name", &snapshot.policy.personalization.display_name, 1024);
                            state.field("tone", &snapshot.policy.personalization.assistant_tone, 16 * 1024);
                        }
                        state.policy = Some(*snapshot);
                    }
                    Output::Session(item) => {
                        if !request.needs_checkpoint() { state.load_session(&item); }
                        self.sessions.updated(item); self.inbox.refresh();
                    }
                    Output::SessionChange(change) => match *change {
                        SessionUpdateResult::Committed { session } => { state.load_session(&session); self.sessions.updated(session); self.inbox.refresh(); }
                        SessionUpdateResult::RevisionConflict { .. } => { state.note = None; state.error = Some(self.i18n.text("controls-conflict")); },
                    },
                    Output::PolicyChange(change) => match change {
                        maka_protocol::configuration::policy::RuntimePolicyMutationResult::Committed { .. } => state.requested = Some(Work::Policy),
                        maka_protocol::configuration::policy::RuntimePolicyMutationResult::RevisionConflict { .. } => { state.note = None; state.error = Some(self.i18n.text("controls-conflict")); },
                    },
                    Output::Executors(result) => match result {
                        maka_client::controls::ExecutorSearchResult::Page { page } => {
                            if let Work::Executors { cursor, .. } = request.work {
                                state.executor_cursor = cursor;
                            }
                            state.executors = Some(page);
                        }
                        maka_client::controls::ExecutorSearchResult::Stale => {
                            state.executors = None;
                            state.executor_cursor = None;
                            state.executor_previous.clear();
                            state.error = Some(self.i18n.text("controls-executors-stale"));
                        }
                    },
                    Output::Turns(turns) => state.turns = Some(turns), Output::Landmarks(landmarks) => state.landmarks = Some(landmarks),
                    Output::Compact(result) => {
                        use maka_protocol::{context::ContextCompactResult, turn::ContextCompactionOutcome};
                        state.note = Some(match *result {
                            ContextCompactResult::Started { .. } => "controls-compact-started",
                            ContextCompactResult::Finished { outcome, .. } => match outcome {
                                ContextCompactionOutcome::Compacted { .. } => "controls-compact-finished",
                                ContextCompactionOutcome::Unchanged { reason } => { state.error = Some(reason); "controls-compact-unchanged" }
                                ContextCompactionOutcome::Failed { reason } => { state.error = Some(reason); "controls-compact-failed" }
                            }
                        });
                        self.sessions.refresh_detail();
                    }
                    Output::CompactStatus(turn) => {
                        use maka_protocol::turn::TurnState;
                        let matching = state.saved.as_ref().is_some_and(|saved| matches!(&saved.mutation,
                            Mutation::Compact(input) if input.session_id == turn.session_id && input.turn_id == turn.turn_id));
                        if matching {
                            state.saved = None;
                            state.note = Some(match &turn.state {
                                TurnState::Admitted(_) | TurnState::Created(_) | TurnState::Running(_) | TurnState::WaitingForUser(_) => "controls-compact-started",
                                TurnState::Completed { context_compaction_outcome: Some(maka_protocol::turn::ContextCompactionOutcome::Compacted { .. }), .. } => "controls-compact-finished",
                                TurnState::Completed { context_compaction_outcome: Some(maka_protocol::turn::ContextCompactionOutcome::Unchanged { .. }), .. } => "controls-compact-unchanged",
                                _ => "controls-compact-failed",
                            });
                            self.sessions.refresh_detail();
                        }
                    }
                    Output::Queue => { self.sessions.refresh_detail(); self.inbox.refresh(); }
                }
            }
            Err(error) => {
                if request.needs_checkpoint() && !work::unknown(&error) {
                    state.saved = None;
                }
                state.note = state.saved.as_ref().map(|_| "controls-unknown");
                state.error = Some(error.to_string());
            }
        }
        if let Some(id) = visit {
            self.apply(Action::Visit(Route::Session(id)));
        }
    }
}
impl State {
    fn queue_mutation(&mut self, mutation: Mutation) {
        let Some(target) = self.target.clone() else {
            return;
        };
        self.saved = Some(Checkpoint {
            target,
            page: self.page,
            mutation: mutation.clone(),
        });
        self.requested = Some(Work::Write(mutation));
        self.error = None;
        self.note = None;
    }
    fn mutation(&self) -> Result<Mutation, String> {
        let target = self
            .target
            .as_ref()
            .ok_or("Missing session control target")?;
        let session = target.session.clone();
        let mutation = match self.page {
            Page::Preferences => Mutation::Policy(RuntimePolicyMutationInput {
                expected_revision: self
                    .policy
                    .as_ref()
                    .ok_or("Preferences are loading")?
                    .revision,
                operation: RuntimePolicyMutation::SetPersonalization {
                    value: Personalization {
                        display_name: self.text("display-name").into(),
                        assistant_tone: self.text("tone").into(),
                    },
                },
            }),
            Page::Metadata => Mutation::Metadata(SessionMetadataUpdateInput {
                session_id: session,
                expected_revision: target.revision,
                patch: SessionMetadataPatch {
                    name: None,
                    labels: (!self.labels_truncated).then(|| {
                        self.text("labels")
                            .lines()
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned)
                            .collect()
                    }),
                    is_flagged: Some(self.flag),
                },
            }),
            Page::Executor => {
                let id = self.executor.as_ref().ok_or("Choose an executor")?;
                let input = serde_json::json!({"sessionId":session,"expectedRevision":target.revision,"patch":{"executorTarget":{"executorId":id,"settings":{"model":(!self.text("model").trim().is_empty()).then(|| self.text("model").trim()),"thinkingLevel":self.thinking}}}});
                // Optional settings are omitted, not serialized as null.
                let mut input = input;
                input["patch"]["executorTarget"]["settings"]
                    .as_object_mut()
                    .unwrap()
                    .retain(|_, v| !v.is_null());
                Mutation::Executor(
                    decode_session_configuration_update_input(&input).map_err(|e| e.to_string())?,
                )
            }
            Page::NewExecutor => {
                let id = self.executor.as_ref().ok_or("Choose an executor")?;
                let settings = maka_runtime::executor::Settings {
                    model: (!self.text("model").trim().is_empty())
                        .then(|| self.text("model").trim().to_owned()),
                    thinking_level: self.thinking,
                };
                let mut input = serde_json::json!({
                    "sessionId":session,
                    "workspace":target.workspace.as_ref().ok_or("Missing creation workspace")?,
                    "executorId":id,"executorSettings":settings,
                });
                if !self.text("name").trim().is_empty() {
                    input["name"] = self.text("name").trim().into();
                }
                Mutation::Create(decode_session_create_input(&input).map_err(|e| e.to_string())?)
            }
            Page::Compact => Mutation::Compact(maka_protocol::context::ContextCompactInput {
                session_id: session,
                turn_id: uuid::Uuid::new_v4().to_string(),
            }),
            Page::MarkRead => Mutation::Read(SessionReadMarkerSetInput {
                session_id: session,
                read_through_message_id: target
                    .read_message
                    .clone()
                    .ok_or("No visible saved message")?,
            }),
            Page::Interrupt => {
                let (turn, run) = target.run.clone().ok_or("No active run")?;
                Mutation::Interrupt(maka_protocol::message::InterruptInput {
                    origin_host_epoch: target.epoch.clone(),
                    session_id: session,
                    interrupt_id: uuid::Uuid::new_v4().to_string(),
                    turn_id: turn,
                    run_id: run,
                })
            }
            Page::RetractQueue => Mutation::Retract(maka_protocol::message::RetractInput {
                origin_host_epoch: target.epoch.clone(),
                session_id: session,
                retract_id: uuid::Uuid::new_v4().to_string(),
            }),
            Page::History => return Err("History is read-only".into()),
        };
        mutation.validate()?;
        Ok(mutation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maka_client::ClientError;
    #[test]
    fn failed_current_state_read_never_forgets_an_unresolved_mutation() {
        let mut app = App::new(
            "/unused".into(),
            crate::i18n::I18n::new(
                crate::LocalePreference::Explicit(crate::Locale::En),
                crate::Locale::En,
            ),
        );
        app.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "epoch".into(),
        };
        let target = Target {
            root: "root".into(),
            epoch: "epoch".into(),
            session: "s".into(),
            name: "Session".into(),
            revision: 1,
            read_message: None,
            run: None,
            workspace: None,
        };
        let saved = Checkpoint {
            target: target.clone(),
            page: Page::Metadata,
            mutation: Mutation::Metadata(SessionMetadataUpdateInput {
                session_id: "s".into(),
                expected_revision: 1,
                patch: SessionMetadataPatch {
                    name: None,
                    labels: None,
                    is_flagged: Some(true),
                },
            }),
        };
        app.session_controls.restore(saved.clone());
        let request = Request {
            target,
            sequence: 1,
            work: Work::Session,
        };
        app.session_controls.pending = Some(request.clone());
        app.session_controls_completed(
            request,
            Err(RequestFailure::NotDispatched(ClientError::Protocol(
                "Read failed".into(),
            ))),
        );
        assert!(app.session_controls.checkpoint().is_some());
        app.session_controls.disconnect();
        assert!(app.session_controls.requested.is_none());
        assert!(app.session_controls.pending.is_none());
        let mut restored = State::default();
        restored.restore(saved);
        assert!(
            restored.requested.is_none(),
            "Restoring is not permission to replay a mutation"
        );
    }

    fn connected(locale: crate::Locale) -> App {
        let mut app = App::new(
            "/unused".into(),
            crate::i18n::I18n::new(crate::LocalePreference::Explicit(locale), locale),
        );
        app.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "epoch".into(),
        };
        app.chrome.motion = false;
        app
    }
    fn choices() -> maka_client::controls::ExecutorChoices {
        serde_json::from_value(serde_json::json!({
            "revision":1,"complete":true,"nextCursor":null,
            "executors":[{"id":"executor","displayName":"Public executor","capabilities":{"thinking":true,"toolActivity":false,"attachments":false,"historyCopy":false}}]
        })).unwrap()
    }
    fn draw(
        app: &mut App,
        width: u16,
        height: u16,
    ) -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::view::draw(frame, app))
            .unwrap();
        terminal
    }
    fn press(app: &mut App, code: crossterm::event::KeyCode) {
        app.input(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        ));
    }
    fn click(app: &mut App, rect: ratatui::layout::Rect) {
        assert!(!rect.is_empty());
        app.input(crossterm::event::Event::Mouse(
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: crossterm::event::KeyModifiers::NONE,
            },
        ));
    }
    fn form(page: Page, locale: crate::Locale) -> App {
        let mut app = connected(locale);
        let item = super::super::sessions::tests::item("session");
        app.session_controls = State {
            target: Some(Target {
                root: "root".into(),
                epoch: "epoch".into(),
                session: item.id.clone(),
                name: item.name.clone(),
                revision: 1,
                read_message: Some("message".into()),
                run: Some(("turn".into(), "run".into())),
                workspace: None,
            }),
            page,
            visible: true,
            ..State::default()
        };
        app.session_controls.load_session(&item);
        if page == Page::Executor {
            app.session_controls.executors = Some(choices());
            app.session_controls.executor = Some("executor".into());
        }
        app
    }
    #[test]
    fn controls_render_real_footers_and_cancel_by_default_in_all_locales() {
        use crossterm::event::KeyCode;
        for locale in crate::Locale::ALL {
            for (width, height) in [(30, 18), (40, 24), (80, 24)] {
                for page in [
                    Page::Metadata,
                    Page::Executor,
                    Page::History,
                    Page::Compact,
                    Page::MarkRead,
                    Page::Interrupt,
                    Page::RetractQueue,
                ] {
                    let mut app = form(page, locale);
                    draw(&mut app, width, height);
                    assert!(
                        app.session_controls.rendered,
                        "{page:?} at {width}x{height} {locale:?}"
                    );
                    assert!(
                        app.layer
                            .rect("footer/close")
                            .is_some_and(|rect| !rect.is_empty())
                    );
                    if page.confirmation() {
                        press(&mut app, KeyCode::Enter);
                        assert!(!app.session_controls.visible);
                        assert!(app.session_controls.checkpoint().is_none());
                    } else if page == Page::Executor {
                        let save = app.layer.rect("footer/save").unwrap();
                        click(&mut app, save);
                        assert!(matches!(
                            app.session_controls.checkpoint().unwrap().mutation,
                            Mutation::Executor(_)
                        ));
                    } else {
                        let close = app.layer.rect("footer/close").unwrap();
                        click(&mut app, close);
                        assert!(!app.session_controls.visible);
                    }
                }
            }
        }
    }
    #[test]
    fn history_body_paging_remains_actionable_when_the_footer_is_narrow() {
        use crossterm::event::KeyCode;
        let mut app = form(Page::History, crate::Locale::En);
        app.session_controls.turns = Some(maka_protocol::navigation::TurnsResult {
            session_id: "session".into(),
            through_sequence: Some(10),
            contributions: vec![],
            next_position: Some(8),
        });
        for _ in 0..12 {
            draw(&mut app, 30, 18);
            if app
                .layer
                .focused_path()
                .is_some_and(|path| path.ends_with("/next"))
            {
                break;
            }
            press(&mut app, KeyCode::Tab);
        }
        assert!(
            app.layer
                .focused_path()
                .is_some_and(|path| path.ends_with("/next"))
        );
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            app.session_controls.requested,
            Some(Work::Turns {
                position: 8,
                through: Some(10)
            })
        ));
    }
    #[test]
    fn home_executor_creation_freezes_identity_and_recovers_without_replay() {
        use crossterm::event::KeyCode;
        let mut app = connected(crate::Locale::En);
        draw(&mut app, 80, 30);
        let entry = app
            .home
            .surface
            .rect("home/center/content/new-executor")
            .expect("ordinary Home entry");
        click(&mut app, entry);
        let original = app.session_controls.target.clone().unwrap();
        assert!(!original.session.is_empty());
        assert!(original.workspace.is_some());
        let request = app.session_controls_request().unwrap();
        assert!(matches!(request.work, Work::Executors { .. }));
        assert_eq!(request.target.session, original.session);
        app.session_controls_completed(
            request,
            Ok(Output::Executors(
                maka_client::controls::ExecutorSearchResult::Page { page: choices() },
            )),
        );
        for _ in 0..16 {
            draw(&mut app, 80, 30);
            if app.layer.focused_path() == Some("content/rows/executor-executor") {
                break;
            }
            press(&mut app, KeyCode::Tab);
        }
        assert_eq!(
            app.layer.focused_path(),
            Some("content/rows/executor-executor")
        );
        press(&mut app, KeyCode::Enter);
        draw(&mut app, 80, 30);
        let create = app.layer.rect("footer/save").unwrap();
        click(&mut app, create);
        let saved = app.session_controls.checkpoint().unwrap();
        assert!(saved.validate("root").is_ok());
        assert!(saved.validate("other").is_err());
        let Mutation::Create(input) = &saved.mutation else {
            panic!("expected executor creation")
        };
        assert_eq!(input.session_id, original.session);
        assert_eq!(Some(&input.workspace), original.workspace.as_ref());
        assert!(matches!(input.target, SessionCreateTarget::Executor { .. }));
        let mut tampered = saved.clone();
        tampered.target.workspace = Some(WorkspaceTarget::HostPath {
            path: "/elsewhere".into(),
        });
        assert!(tampered.validate("root").is_err());
        let wire = serde_json::to_value(&saved).unwrap();
        assert!(wire["mutation"]["input"].get("modelTarget").is_none());
        assert_eq!(wire["mutation"]["input"]["executorId"], "executor");
        let write = app.session_controls_request().unwrap();
        assert!(write.needs_checkpoint());
        assert!(app.session_controls_after_checkpoint(&write, &Ok(())));
        app.session_controls_completed(
            write,
            Err(RequestFailure::Unknown(ClientError::Protocol(
                "response lost".into(),
            ))),
        );
        assert!(app.session_controls.checkpoint().is_some());
        app.session_controls.disconnect();
        let mut restored = connected(crate::Locale::En);
        restored.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "next-epoch".into(),
        };
        restored
            .session_controls
            .restore(serde_json::from_value(wire).unwrap());
        assert!(restored.session_controls_request().is_none());
        restored.session_controls_action(Command::Reopen);
        assert_eq!(
            restored.session_controls.target.as_ref().unwrap().session,
            original.session
        );
        assert_eq!(
            restored.session_controls.checkpoint().unwrap().target.epoch,
            "epoch"
        );
        draw(&mut restored, 80, 30);
        restored.session_controls_action(Command::Refresh);
        let read = restored.session_controls_request().unwrap();
        assert!(matches!(read.work, Work::CreationStatus));
        assert!(!read.needs_checkpoint());
        assert_eq!(read.target.session, original.session);
        restored.session_controls_completed(read, Ok(Output::CreationStatus(None)));
        assert!(restored.session_controls.checkpoint().is_some());
        assert!(!restored.session_controls_offered(&Command::Save));
        draw(&mut restored, 80, 30);
        restored.session_controls_action(Command::Refresh);
        let read = restored.session_controls_request().unwrap();
        restored.session_controls_completed(
            read,
            Ok(Output::CreationStatus(Some(Box::new(
                super::super::sessions::tests::item(&original.session),
            )))),
        );
        assert!(
            restored.session_controls.checkpoint().is_some(),
            "current state cannot settle a historical creation"
        );
        assert_eq!(restored.navigation.current(), Route::Workspace);
        draw(&mut restored, 80, 30);
        restored.session_controls_action(Command::OpenCreated);
        assert_eq!(
            restored.navigation.current(),
            Route::Session(original.session)
        );
        assert!(restored.session_controls.checkpoint().is_some());
    }
    #[test]
    fn project_executor_menu_keeps_the_clicked_project_after_selection_changes() {
        use crossterm::event::KeyCode;
        let mut app = connected(crate::Locale::En);
        app.apply(Action::Visit(Route::Projects));
        app.projects.query().unwrap();
        app.projects.loading = false;
        app.projects.loaded = true;
        app.projects.items = ["project-a", "project-b"]
            .map(|id| super::super::projects::Item {
                id: id.into(),
                name: id.into(),
                archived: false,
                available: true,
            })
            .into();
        app.projects.selected = Some("project-a".into());
        let global = app.new_executor_session_action().unwrap();
        assert!(matches!(
            global,
            Action::SessionControls(Command::Open(
                Target {
                    workspace: Some(WorkspaceTarget::HostPath { .. }),
                    ..
                },
                Page::NewExecutor
            ))
        ));
        draw(&mut app, 80, 30);
        let menu = app
            .projects
            .surface
            .rect("projects/rows/project-a/actions")
            .unwrap();
        click(&mut app, menu);
        draw(&mut app, 80, 30);
        app.projects.selected = Some("project-b".into());
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.session_controls.page, Page::NewExecutor);
        assert_eq!(
            app.session_controls.target.as_ref().unwrap().workspace,
            Some(WorkspaceTarget::Project {
                project_id: "project-a".into()
            })
        );
        let request = app.session_controls_request().unwrap();
        app.session_controls_completed(
            request,
            Ok(Output::Executors(
                maka_client::controls::ExecutorSearchResult::Page { page: choices() },
            )),
        );
        draw(&mut app, 80, 30);
        app.session_controls_action(Command::Executor("executor".into()));
        draw(&mut app, 80, 30);
        let save = app.layer.rect("footer/save").unwrap();
        click(&mut app, save);
        let saved = app.session_controls.checkpoint().unwrap();
        assert!(saved.validate("root").is_ok());
        let Mutation::Create(input) = saved.mutation else {
            panic!("creation expected")
        };
        assert_eq!(
            input.workspace,
            WorkspaceTarget::Project {
                project_id: "project-a".into()
            }
        );
    }
}
