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

//! Native session transfer. Paths belong to Host; local files are never opened.
mod saved;
#[cfg(test)]
mod tests;
mod view;
pub use saved::Checkpoint;
pub(crate) use view::{draw_fields, sheet};

use crate::{
    app::{Action, App, ConnectionState},
    editor::Editor,
    navigation::Route,
    pages::projects::Projects,
};
use maka_client::{Client, ClientError, RequestFailure};
use maka_protocol::{
    OperationErrorCode, project,
    session::{WorkspaceTarget, bundle as api},
};
use saved::{Frozen, Intent, Mode, Outcome, Receipt, Workspace};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    root: String,
    epoch: String,
    session: Option<String>,
    name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Export(Target),
    Import(Target),
    Resume,
    Close,
    Preview,
    Review,
    Edit,
    Confirm,
    Query,
    Visit,
    HostPath,
    Projects,
    SelectProject(String),
    RefreshProjects,
    PreviousProjects,
    NextProjects,
    Forget,
    Keep,
    ConfirmForget,
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Export(_) => "bundle-export",
            Self::Import(_) => "bundle-import",
            Self::Resume => "bundle-resume",
            Self::Close => "session-remove-close",
            Self::Preview => "bundle-preview",
            Self::Review => "bundle-review",
            Self::Edit => "bundle-edit",
            Self::Confirm => "bundle-confirm",
            Self::Query => "bundle-query",
            Self::Visit => "bundle-visit",
            Self::HostPath => "bundle-workspace-path",
            Self::Projects => "bundle-workspace-project",
            Self::SelectProject(_) => "project-select",
            Self::RefreshProjects => "command-refresh",
            Self::PreviousProjects => "sessions-previous",
            Self::NextProjects => "sessions-next",
            Self::Forget => "bundle-forget",
            Self::Keep => "session-cancel",
            Self::ConfirmForget => "bundle-forget-confirm",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Request {
    root: String,
    epoch: String,
    sequence: u64,
    kind: RequestKind,
}
impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.epoch == other.epoch && self.sequence == other.sequence
    }
}
impl Request {
    pub fn needs_checkpoint(&self) -> bool {
        matches!(self.kind, RequestKind::Write(_))
    }
}
#[derive(Clone, Debug)]
enum RequestKind {
    Preview(String),
    ImportPreview(api::ImportPreview),
    Query(Frozen),
    Projects(project::Query),
    Write(Frozen),
}
#[derive(Debug)]
pub enum Output {
    Preview(api::Previewed),
    ImportPreview(api::ImportPreviewed),
    Queried(api::ImportQueried),
    Projects(project::QueryResult),
    Exported(api::Exported),
    Imported(api::Imported),
}

pub struct State {
    pub visible: bool,
    rendered: bool,
    intent: Option<Intent>,
    path: Editor,
    workspace: Editor,
    project: Option<(String, String)>,
    choosing: bool,
    projects: Projects,
    reviewing: bool,
    forgetting: bool,
    outcome: Outcome,
    preview: Option<api::Previewed>,
    import_preview: Option<api::ImportPreviewed>,
    error: Option<&'static str>,
    detail: Option<String>,
    pending: Option<Request>,
    requested: Option<RequestKind>,
    sequence: u64,
    saving: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            visible: false,
            rendered: false,
            intent: None,
            path: Editor::bounded(4096, "bundle-path-invalid"),
            workspace: Editor::bounded(4096, "bundle-path-invalid"),
            project: None,
            choosing: false,
            projects: Projects::default(),
            reviewing: false,
            forgetting: false,
            outcome: Outcome::Draft,
            preview: None,
            import_preview: None,
            error: None,
            detail: None,
            pending: None,
            requested: None,
            sequence: 0,
            saving: false,
        }
    }
}
impl State {
    pub fn invalidate_geometry(&mut self) {
        self.rendered = false;
        self.path.invalidate_geometry();
        self.workspace.invalidate_geometry();
    }
    pub(crate) fn presented(&mut self, shown: bool) {
        self.rendered = shown;
    }
    pub fn disconnect(&mut self) {
        self.pending = None;
        self.requested = None;
        self.saving = false;
        self.preview = None;
        self.import_preview = None;
        self.projects = Projects::default();
        self.reviewing = false;
        self.invalidate_geometry();
    }
    fn editable(&self) -> bool {
        matches!(self.outcome, Outcome::Draft) && self.pending.is_none() && self.requested.is_none()
    }
    fn import(&self) -> bool {
        self.intent
            .as_ref()
            .is_some_and(|intent| matches!(intent.mode, Mode::Import))
    }
    fn freeze(&self) -> Option<Frozen> {
        let intent = self.intent.as_ref()?;
        let path = self.path.text().to_owned();
        if !saved::valid_path(&path) {
            return None;
        }
        match &intent.mode {
            Mode::Export { session, .. } => {
                let preview = self.preview.as_ref()?;
                Some(Frozen::Export {
                    session: session.clone(),
                    destination: path,
                    digest: preview.subtree_digest.clone(),
                    count: preview.session_count,
                })
            }
            Mode::Import => Some(Frozen::Import {
                source: path,
                workspace: self.selected_workspace()?,
                expected: self.import_preview.clone()?,
            }),
        }
    }
    fn selected_workspace(&self) -> Option<Workspace> {
        if let Some((id, name)) = &self.project {
            Some(Workspace::Project {
                id: id.clone(),
                name: name.clone(),
            })
        } else if saved::valid_path(self.workspace.text()) {
            Some(Workspace::HostPath {
                path: self.workspace.text().to_owned(),
            })
        } else {
            None
        }
    }
    fn import_preview_input(&self) -> Option<api::ImportPreview> {
        if !self.import() || !saved::valid_path(self.path.text()) {
            return None;
        }
        Some(api::ImportPreview {
            source: self.path.text().to_owned(),
            workspace: self.selected_workspace()?.target(),
        })
    }
}

pub async fn execute(client: &Client, request: &Request) -> Result<Output, RequestFailure> {
    match &request.kind {
        RequestKind::Preview(session) => client
            .preview_session_bundle(api::Preview {
                session_id: session.clone(),
            })
            .await
            .map(Output::Preview),
        RequestKind::ImportPreview(input) => client
            .preview_session_bundle_import(input.clone())
            .await
            .map(Output::ImportPreview),
        RequestKind::Query(Frozen::Import { expected, .. }) => client
            .query_session_bundle_import(api::ImportQuery {
                bundle_digest: expected.bundle_digest.clone(),
                binding_digest: expected.binding_digest.clone(),
            })
            .await
            .map(Output::Queried),
        RequestKind::Query(Frozen::Export { .. }) => unreachable!("export has no receipt query"),
        RequestKind::Projects(query) => client
            .project_catalog(query.clone())
            .await
            .map(Output::Projects),
        RequestKind::Write(Frozen::Export {
            session,
            destination,
            digest,
            ..
        }) => client
            .export_session_bundle(api::Export {
                session_id: session.clone(),
                destination: destination.clone(),
                expected_subtree_digest: Some(digest.clone()),
            })
            .await
            .map(Output::Exported),
        RequestKind::Write(Frozen::Import {
            source,
            workspace,
            expected,
        }) => client
            .import_session_bundle(api::Import {
                source: source.clone(),
                workspace: workspace.target(),
                expected: expected.clone(),
            })
            .await
            .map(Output::Imported),
    }
}

impl App {
    fn bundle_target(&self, export: bool) -> Option<Target> {
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        let (session, name) = if export {
            let Route::Session(id) = self.navigation.current() else {
                return None;
            };
            if self.chat.removed {
                return None;
            }
            let name = self
                .sessions
                .items
                .iter()
                .find(|item| item.id == id)
                .map_or_else(|| id.clone(), |item| item.name.clone());
            (Some(id), name)
        } else {
            (None, String::new())
        };
        Some(Target {
            root: root_id.clone(),
            epoch: epoch.clone(),
            session,
            name,
        })
    }
    pub fn bundle_export_action(&self) -> Option<Action> {
        if self.bundle.intent.is_some() {
            return Some(Action::Bundle(Command::Resume));
        }
        self.bundle_target(true)
            .map(|target| Action::Bundle(Command::Export(target)))
    }
    pub fn bundle_import_action(&self) -> Option<Action> {
        if self.bundle.intent.is_some() {
            return Some(Action::Bundle(Command::Resume));
        }
        self.bundle_target(false)
            .map(|target| Action::Bundle(Command::Import(target)))
    }
    pub fn bundle_commands(&self) -> Vec<(Action, &'static str)> {
        let mut commands = vec![];
        if let Some(action) = self.bundle_import_action() {
            let key = if self.bundle.intent.is_some() {
                "bundle-resume"
            } else {
                "bundle-import"
            };
            commands.push((action, key));
        }
        if self.bundle.intent.is_none()
            && let Some(action) = self.bundle_export_action()
        {
            commands.push((action, "bundle-export"));
        }
        commands
    }
    fn bundle_connected(&self) -> bool {
        matches!((&self.connection, &self.bundle.intent),
            (ConnectionState::Connected { root_id, .. }, Some(intent)) if *root_id == intent.root)
    }
    pub fn bundle_enabled(&self, command: &Command) -> bool {
        let state = &self.bundle;
        match command {
            Command::Export(target) => {
                !state.visible
                    && state.intent.is_none()
                    && self.bundle_target(true).as_ref() == Some(target)
            }
            Command::Import(target) => {
                !state.visible
                    && state.intent.is_none()
                    && self.bundle_target(false).as_ref() == Some(target)
            }
            Command::Resume => !state.visible && state.intent.is_some() && self.bundle_connected(),
            Command::Close => state.visible,
            Command::Keep => state.visible && state.forgetting,
            _ if !state.visible || !state.rendered || !self.bundle_connected() => false,
            Command::Forget => {
                state.pending.is_none() && state.requested.is_none() && !state.forgetting
            }
            Command::ConfirmForget => {
                state.forgetting && state.pending.is_none() && state.requested.is_none()
            }
            Command::Query => {
                !state.forgetting
                    && state.pending.is_none()
                    && state.requested.is_none()
                    && matches!(
                        state.outcome,
                        Outcome::Unknown {
                            write: Frozen::Import { .. }
                        }
                    )
            }
            Command::Visit => {
                !state.forgetting
                    && matches!(
                        state.outcome,
                        Outcome::Complete {
                            receipt: Receipt::Imported { .. },
                            ..
                        }
                    )
            }
            _ if !state.editable() || state.forgetting => false,
            Command::Preview => !state.import() && !state.reviewing,
            Command::Review => {
                !state.reviewing
                    && !state.choosing
                    && if state.import() {
                        state.import_preview_input().is_some()
                    } else {
                        state.freeze().is_some()
                    }
            }
            Command::Edit => state.reviewing || state.choosing,
            Command::Confirm => state.reviewing && state.freeze().is_some(),
            Command::HostPath | Command::Projects => state.import() && !state.reviewing,
            Command::SelectProject(id) => {
                state.choosing
                    && state.projects.ready()
                    && state
                        .projects
                        .items
                        .iter()
                        .any(|item| item.id == *id && item.usable())
            }
            Command::RefreshProjects => state.choosing && !state.projects.loading,
            Command::PreviousProjects => state.choosing && state.projects.can_previous(),
            Command::NextProjects => {
                state.choosing && state.projects.ready() && state.projects.can_next()
            }
        }
    }
    pub fn bundle_action(&mut self, command: Command) -> Option<Action> {
        if !self.bundle_enabled(&command) {
            return None;
        }
        let state = &mut self.bundle;
        state.error = None;
        state.detail = None;
        match command {
            Command::Export(target) | Command::Import(target) => {
                let mode = match target.session {
                    Some(session) => Mode::Export {
                        session,
                        name: target.name,
                    },
                    None => Mode::Import,
                };
                state.intent = Some(Intent {
                    root: target.root,
                    mode,
                });
                state.visible = true;
                state.rendered = false;
                if let Some(Intent {
                    mode: Mode::Export { session, .. },
                    ..
                }) = &state.intent
                {
                    state.requested = Some(RequestKind::Preview(session.clone()));
                }
            }
            Command::Resume => {
                // Reopening a confirmation starts on Close, even if it was
                // previously dismissed while its write button held focus.
                state.sequence += 1;
                state.visible = true;
                state.rendered = false;
            }
            Command::Close => {
                state.visible = false;
                state.invalidate_geometry();
            }
            Command::Preview => {
                let Mode::Export { session, .. } = &state.intent.as_ref()?.mode else {
                    return None;
                };
                state.preview = None;
                state.requested = Some(RequestKind::Preview(session.clone()));
            }
            Command::Review => {
                state.sequence += 1;
                state.path.error = None;
                state.workspace.error = None;
                if state.import() {
                    state.import_preview = None;
                    state.requested =
                        Some(RequestKind::ImportPreview(state.import_preview_input()?));
                } else {
                    state.reviewing = true;
                }
                state.rendered = false;
            }
            Command::Query => {
                let Outcome::Unknown { write } = &state.outcome else {
                    return None;
                };
                state.requested = Some(RequestKind::Query(write.clone()));
            }
            Command::Visit => {
                let Outcome::Complete {
                    receipt: Receipt::Imported { receipt, .. },
                    ..
                } = &state.outcome
                else {
                    return None;
                };
                let session = receipt.root_session_id.clone();
                state.visible = false;
                return self.apply(Action::Visit(Route::Session(session)));
            }
            Command::Edit => {
                state.reviewing = false;
                state.choosing = false;
                state.rendered = false;
            }
            Command::Confirm => {
                let frozen = state.freeze()?;
                // Saved before dispatch. Recovery conservatively assumes admission.
                state.outcome = Outcome::Unknown {
                    write: frozen.clone(),
                };
                state.requested = Some(RequestKind::Write(frozen));
            }
            Command::HostPath => {
                state.import_preview = None;
                state.project = None;
                state.choosing = false;
            }
            Command::Projects => {
                state.choosing = true;
                state.projects.restart();
            }
            Command::SelectProject(id) => {
                state.import_preview = None;
                let item = state.projects.items.iter().find(|item| item.id == id)?;
                state.project = Some((item.id.clone(), item.name.clone()));
                state.choosing = false;
                state.rendered = false;
            }
            Command::RefreshProjects => state.projects.restart(),
            Command::PreviousProjects => state.projects.change_page(false),
            Command::NextProjects => state.projects.change_page(true),
            Command::Forget => {
                state.sequence += 1;
                state.forgetting = true;
                state.rendered = false;
            }
            Command::Keep => {
                state.forgetting = false;
                state.rendered = false;
            }
            Command::ConfirmForget => {
                let sequence = state.sequence;
                *state = State::default();
                state.sequence = sequence;
            }
        }
        None
    }
    pub fn bundle_catalog_changed(&mut self) {
        if self.bundle.choosing {
            self.bundle.projects.refresh();
        }
    }
    pub fn bundle_request(&mut self) -> Option<Request> {
        if self.closing || !self.bundle_connected() {
            return None;
        }
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        let state = &mut self.bundle;
        if state.pending.is_some() {
            return None;
        }
        let kind = state.requested.take().or_else(|| {
            if state.visible && state.choosing && state.editable() {
                state.projects.query().map(RequestKind::Projects)
            } else {
                None
            }
        })?;
        state.sequence += 1;
        let request = Request {
            root: root_id.clone(),
            epoch: epoch.clone(),
            sequence: state.sequence,
            kind,
        };
        state.saving = request.needs_checkpoint();
        state.pending = Some(request.clone());
        Some(request)
    }
    pub fn bundle_after_checkpoint(
        &mut self,
        request: &Request,
        result: &Result<(), String>,
    ) -> bool {
        let state = &mut self.bundle;
        if !state.saving || state.pending.as_ref() != Some(request) {
            return false;
        }
        state.saving = false;
        if result.is_ok()
            && !self.closing
            && matches!(&self.connection,
            ConnectionState::Connected { root_id, epoch } if *root_id == request.root && *epoch == request.epoch)
        {
            return true;
        }
        state.pending = None;
        state.outcome = Outcome::Draft;
        state.reviewing = false;
        state.error = Some("bundle-save-failed");
        false
    }
    pub fn bundle_completed(&mut self, request: Request, result: Result<Output, RequestFailure>) {
        if self.bundle.pending.as_ref() != Some(&request) {
            return;
        }
        self.bundle.pending = None;
        if !matches!(&self.connection, ConnectionState::Connected { root_id, epoch }
            if *root_id == request.root && *epoch == request.epoch)
        {
            return;
        }
        let state = &mut self.bundle;
        match (request.kind, result) {
            (RequestKind::Preview(_), Ok(Output::Preview(preview))) => {
                state.preview = Some(preview);
                state.error = None;
            }
            (RequestKind::Projects(_), Ok(Output::Projects(output))) => {
                state.projects.complete(Ok(output))
            }
            (RequestKind::Projects(_), Err(error)) => {
                state.projects.complete(Err(error.to_string()));
            }
            (RequestKind::Write(write @ Frozen::Export { .. }), Ok(Output::Exported(receipt))) => {
                if matches!(&write, Frozen::Export { count, .. } if *count == receipt.session_count)
                {
                    state.outcome = Outcome::Complete {
                        write,
                        receipt: Receipt::Exported {
                            count: receipt.session_count,
                            bytes: receipt.compressed_bytes,
                        },
                    };
                } else {
                    state.error = Some("bundle-receipt-invalid");
                }
                state.reviewing = false;
            }
            (RequestKind::ImportPreview(_), Ok(Output::ImportPreview(preview))) => {
                state.import_preview = Some(preview);
                state.reviewing = true;
                state.rendered = false;
            }
            (RequestKind::Write(write @ Frozen::Import { .. }), Ok(Output::Imported(receipt))) => {
                let artifacts = receipt.artifact_files;
                state.outcome = Outcome::Complete {
                    write,
                    receipt: Receipt::Imported {
                        receipt: api::ImportReceipt {
                            root_session_id: receipt.root_session_id,
                            session_ids: receipt.session_ids,
                        },
                        artifacts: Some(artifacts),
                    },
                };
                state.reviewing = false;
                self.sessions.refresh();
                self.inbox.refresh();
            }
            (RequestKind::Query(write @ Frozen::Import { .. }), Ok(Output::Queried(output))) => {
                if let Some(receipt) = output.receipt {
                    if matches!(&write, Frozen::Import { expected, .. } if expected.session_count == receipt.session_ids.len() as u64)
                    {
                        state.outcome = Outcome::Complete {
                            write,
                            receipt: Receipt::Imported {
                                receipt,
                                artifacts: None,
                            },
                        };
                        self.sessions.refresh();
                        self.inbox.refresh();
                    } else {
                        state.error = Some("bundle-receipt-invalid");
                    }
                } else {
                    state.error = Some("bundle-no-receipt");
                }
            }
            (RequestKind::Write(_), Err(error)) => {
                if !uncertain(&error) {
                    state.outcome = Outcome::Draft;
                }
                state.error = Some("bundle-failed");
                state.detail = Some(error.to_string());
                state.reviewing = false;
                // Even a known failure requires a fresh subtree preview before a new export.
                state.preview = None;
                state.import_preview = None;
            }
            (_, Err(error)) => {
                state.error = Some("bundle-failed");
                state.detail = Some(error.to_string());
            }
            _ => {
                state.error = Some("bundle-receipt-invalid");
            }
        }
    }
}

/// Publication can precede an explicit storage/cleanup/catalog error as well as
/// a lost reply. No public read-only bundle receipt exists, so none is replayed.
fn uncertain(error: &RequestFailure) -> bool {
    match error {
        RequestFailure::Unknown(_) => true,
        RequestFailure::Rejected(ClientError::Rejected(error)) => matches!(
            error.code,
            OperationErrorCode::CommitOutcomeUnknown
                | OperationErrorCode::PersistenceFailed
                | OperationErrorCode::InternalFailure
        ),
        _ => false,
    }
}
