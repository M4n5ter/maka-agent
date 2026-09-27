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

mod headers;
mod overlay;
pub mod provider;
mod proxy;
mod view;
use super::{Command as Manage, Entity, Kind, Target, connection};
use crate::{
    app::{Action, App, ConnectionState},
    editor::Editor,
};
use maka_client::{Client, ClientError, RequestFailure};
use maka_protocol::configuration::{
    policy::{ProxyProtocol, RuntimePolicySnapshot, network_test, network_update},
    *,
};
pub(super) use view::{draw, input, sheet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretAction {
    Keep,
    Replace,
    Delete,
}
impl SecretAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "preferences-secret-keep",
            Self::Replace => "preferences-secret-replace",
            Self::Delete => "preferences-secret-delete",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Field(usize),
    Choice(usize, usize),
    Enabled,
    Authentication,
    Protocol(ProxyProtocol),
    Secret(SecretAction),
    Reload,
    Save,
    Test,
    AddHeader,
    ClearOverlay,
    HeaderSecret(usize, SecretAction),
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Reload => "credential-retry",
            Self::Save => "session-save",
            Self::Test => "proxy-test",
            Self::AddHeader => "headers-add",
            Self::ClearOverlay => "request-overlay-clear",
            _ => "connection-preferences",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    target: Target,
    generation: u64,
    operation: Operation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    LoadProxy,
    SaveProxy,
    TestProxy,
    SaveProvider,
    LoadHeaders,
    SaveHeaders,
}
pub struct Request {
    ticket: Ticket,
    work: Work,
}
impl Request {
    pub fn ticket(&self) -> Ticket {
        self.ticket.clone()
    }
}
enum Work {
    LoadProxy,
    SaveProxy(network_update::Update),
    TestProxy(network_test::Input),
    SaveProvider(UpdateCatalogConnectionInput),
    LoadHeaders(String),
    SaveHeaders(maka_protocol::configuration::headers::RequestHeadersReplace),
}
pub enum Response {
    Proxy(RuntimePolicySnapshot, CredentialStatus),
    ProxySaved(network_update::UpdateResult),
    ProxyTested(network_test::Output),
    ProviderSaved(CatalogMutationResult),
    HeadersLoaded(maka_protocol::configuration::headers::RequestHeadersQueryResult),
    HeadersSaved(maka_protocol::configuration::headers::RequestHeadersReplaceResult),
}
pub(super) struct State {
    generation: u64,
    requested: Option<Work>,
    body: Body,
    saved: bool,
}
enum Body {
    LoadingProxy,
    Proxy(Box<proxy::Proxy>),
    Provider(provider::Form),
    LoadingHeaders,
    Headers(headers::Headers),
    Overlay(Box<overlay::Overlay>),
}
impl State {
    pub fn new(app: &App, target: &Target, kind: Kind, generation: u64) -> Option<Self> {
        let (body, requested) = if kind == Kind::NetworkProxy {
            (Body::LoadingProxy, Some(Work::LoadProxy))
        } else if kind == Kind::Connection(connection::Change::Preferences) {
            let Entity::Connection(row) = &target.entity else {
                return None;
            };
            let descriptor = app.providers.find(&row.provider).map(|p| &p.descriptor);
            (
                Body::Provider(provider::Form::new(&row.configuration, descriptor)),
                None,
            )
        } else if kind == Kind::Connection(connection::Change::RequestHeaders) {
            let Entity::Connection(row) = &target.entity else {
                return None;
            };
            (
                Body::LoadingHeaders,
                Some(Work::LoadHeaders(row.id.clone())),
            )
        } else if kind == Kind::Connection(connection::Change::RequestBodyOverlay) {
            let Entity::Connection(row) = &target.entity else {
                return None;
            };
            (
                Body::Overlay(Box::new(overlay::Overlay::new(
                    row.request_body_overlay.clone(),
                ))),
                None,
            )
        } else {
            return None;
        };
        Some(Self {
            generation,
            requested,
            body,
            saved: false,
        })
    }
    pub fn invalidate_geometry(&mut self) {
        match &mut self.body {
            Body::Proxy(p) => {
                for e in &mut p.fields {
                    e.invalidate_geometry();
                }
            }
            Body::Provider(f) => f.invalidate_geometry(),
            Body::Headers(h) => h.invalidate_geometry(),
            Body::Overlay(o) => o.editor.invalidate_geometry(),
            Body::LoadingProxy | Body::LoadingHeaders => {}
        }
    }
    fn editor(&self, index: usize) -> Option<&Editor> {
        match &self.body {
            Body::Proxy(p) => p.fields.get(index).filter(|_| {
                index != 3 || p.value.auth_enabled && p.secret_action == SecretAction::Replace
            }),
            Body::Provider(f) => f.fields.get(index)?.editor.as_ref(),
            Body::Overlay(o) => (index == 0).then_some(&o.editor),
            Body::Headers(h) => {
                let row = h.rows.get(index / 2)?;
                if row.action == SecretAction::Delete {
                    None
                } else if index.is_multiple_of(2) {
                    Some(&row.name)
                } else {
                    (row.action == SecretAction::Replace).then_some(&row.value)
                }
            }
            _ => None,
        }
    }
    fn editor_mut(&mut self, index: usize) -> Option<&mut Editor> {
        match &mut self.body {
            Body::Proxy(p) => {
                let allowed =
                    index != 3 || p.value.auth_enabled && p.secret_action == SecretAction::Replace;
                p.fields.get_mut(index).filter(|_| allowed)
            }
            Body::Provider(f) => f.fields.get_mut(index)?.editor.as_mut(),
            Body::Overlay(o) => (index == 0).then_some(&mut o.editor),
            Body::Headers(h) => {
                let row = h.rows.get_mut(index / 2)?;
                if row.action == SecretAction::Delete {
                    None
                } else if index.is_multiple_of(2) {
                    Some(&mut row.name)
                } else {
                    (row.action == SecretAction::Replace).then_some(&mut row.value)
                }
            }
            _ => None,
        }
    }
    fn clear_secrets(&mut self) {
        if let Body::Headers(h) = &mut self.body {
            h.clear_secrets();
        }
        if let Body::Proxy(p) = &mut self.body {
            p.fields[3] = Editor::bounded(10_240, "connection-preferences-limit");
        }
    }
}
fn basis(status: &CredentialStatus) -> Option<CredentialVersionBasis> {
    match &status.state {
        CredentialState::Absent => None,
        CredentialState::Configured {
            credential_id,
            revision,
            ..
        } => Some(CredentialVersionBasis {
            locator: status.locator.clone(),
            credential_id: credential_id.clone(),
            revision: *revision,
        }),
    }
}

pub async fn execute(client: &Client, request: &Request) -> Result<Response, RequestFailure> {
    match &request.work {
        Work::LoadProxy => {
            let policy = client.runtime_policy().await?;
            let CredentialVaultQueryResult::Status { status } =
                client.credential_status(network_update::locator()).await?
            else {
                return Err(RequestFailure::Unknown(ClientError::Protocol(
                    "Missing proxy credential status".into(),
                )));
            };
            Ok(Response::Proxy(policy, status))
        }
        Work::SaveProxy(update) => client
            .update_network_proxy(update.clone())
            .await
            .map(Response::ProxySaved),
        Work::TestProxy(input) => client
            .test_network_proxy(input.clone())
            .await
            .map(Response::ProxyTested),
        Work::SaveProvider(input) => client
            .update_connection(input.clone())
            .await
            .map(Response::ProviderSaved),
        Work::LoadHeaders(id) => client
            .request_headers(id)
            .await
            .map(Response::HeadersLoaded),
        Work::SaveHeaders(input) => client
            .replace_request_headers(input)
            .await
            .map(Response::HeadersSaved),
    }
}
impl App {
    pub fn network_proxy_action(&self) -> Option<Action> {
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        Some(Action::Manage(Manage::Open(
            Target {
                root: root_id.clone(),
                epoch: epoch.clone(),
                name: String::new(),
                entity: Entity::NetworkProxy,
            },
            Kind::NetworkProxy,
        )))
    }
    pub(super) fn preferences_enabled(&self, command: &Command) -> bool {
        let Some(dialog) = &self.management.dialog else {
            return false;
        };
        let Some(state) = &dialog.preferences else {
            return false;
        };
        if !dialog.visible
            || !self.management_identity(&dialog.target)
            || self.management.preferences_pending.is_some()
            || state.requested.is_some()
        {
            return false;
        }
        if matches!(command, Command::Reload) {
            return matches!(
                state.body,
                Body::LoadingProxy | Body::Proxy(_) | Body::LoadingHeaders | Body::Headers(_)
            );
        }
        if dialog.blocked {
            return false;
        }
        match (&state.body, command) {
            (_, Command::Field(index)) => state.editor(*index).is_some(),
            (Body::Proxy(p), Command::Save) => p.changed(),
            (Body::Proxy(p), Command::Test) => p.test_input().is_ok(),
            (
                Body::Proxy(_),
                Command::Enabled
                | Command::Authentication
                | Command::Protocol(_)
                | Command::Secret(_),
            ) => true,
            (Body::Provider(f), Command::Choice(index, choice)) => f
                .fields
                .get(*index)
                .is_some_and(|field| *choice < field.choices.len()),
            (Body::Provider(f), Command::Save) => f.changed(),
            (Body::Headers(h), Command::Save) => h.changed(),
            (Body::Overlay(o), Command::Save) => o.changed(),
            (Body::Overlay(o), Command::ClearOverlay) => !o.editor.text().is_empty(),
            (Body::Headers(_), Command::AddHeader) => true,
            (Body::Headers(h), Command::HeaderSecret(index, _)) => *index < h.rows.len(),
            _ => false,
        }
    }
    pub(super) fn preferences_action(&mut self, command: Command) -> Option<Action> {
        if !self.preferences_enabled(&command) {
            return None;
        }
        if let Command::Field(index) = command {
            self.layer.focus_path(&view::row_path(index));
            return None;
        }
        let dialog = self.management.dialog.as_mut()?;
        let state = dialog.preferences.as_mut()?;
        state.saved = false;
        dialog.error = None;
        match command {
            Command::Reload => {
                state.clear_secrets();
                if let Entity::Connection(row) = &dialog.target.entity {
                    state.body = Body::LoadingHeaders;
                    state.requested = Some(Work::LoadHeaders(row.id.clone()));
                } else {
                    state.body = Body::LoadingProxy;
                    state.requested = Some(Work::LoadProxy);
                }
                dialog.blocked = false;
            }
            Command::Save => {
                state.requested = Some(match &state.body {
                    Body::Proxy(p) => Work::SaveProxy(p.update().ok()?),
                    Body::Headers(h) => Work::SaveHeaders(h.update().ok()?),
                    Body::Overlay(o) => {
                        let Entity::Connection(row) = &dialog.target.entity else {
                            return None;
                        };
                        let mut update = connection::update(row, Kind::Rename, &row.name);
                        update.changes.request_body_overlay = o.patch().ok()?;
                        Work::SaveProvider(update)
                    }
                    Body::Provider(f) => {
                        let Entity::Connection(row) = &dialog.target.entity else {
                            return None;
                        };
                        let mut input = connection::update(row, Kind::Rename, &row.name);
                        input.changes.configuration = f.value().ok()?;
                        let wire = serde_json::to_value(&input).ok()?;
                        match maka_protocol::configuration::decode_update_connection_input(&wire) {
                            Ok(input) => Work::SaveProvider(input),
                            Err(_) => {
                                dialog.error = Some("connection-preferences-invalid");
                                return None;
                            }
                        }
                    }
                    _ => return None,
                });
            }
            Command::Test => {
                let Body::Proxy(p) = &state.body else {
                    return None;
                };
                state.requested = Some(Work::TestProxy(p.test_input().ok()?));
            }
            Command::ClearOverlay => {
                if let Body::Overlay(o) = &mut state.body {
                    o.editor = Editor::bounded(32 * 1024, "connection-preferences-limit");
                }
            }
            Command::AddHeader => {
                if let Body::Headers(h) = &mut state.body {
                    h.rows.push(headers::Row::new(None));
                }
            }
            Command::HeaderSecret(index, action) => {
                if let Body::Headers(h) = &mut state.body
                    && let Some(row) = h.rows.get_mut(index)
                {
                    row.action = action;
                    row.value = Editor::bounded(16 * 1024, "connection-preferences-limit");
                }
            }
            Command::Choice(index, choice) => {
                if let Body::Provider(f) = &mut state.body {
                    f.choose(index, choice);
                }
            }
            command => {
                if let Body::Proxy(p) = &mut state.body {
                    p.test = None;
                    match command {
                        Command::Enabled => p.value.enabled = !p.value.enabled,
                        Command::Authentication => {
                            p.value.auth_enabled = !p.value.auth_enabled;
                            if !p.value.auth_enabled {
                                p.secret_action = SecretAction::Delete;
                                p.fields[3] =
                                    Editor::bounded(10_240, "connection-preferences-limit");
                            }
                        }
                        Command::Protocol(protocol) => p.value.protocol = protocol,
                        Command::Secret(action) => {
                            p.secret_action = action;
                            p.fields[3] = Editor::bounded(10_240, "connection-preferences-limit");
                        }
                        _ => {}
                    }
                }
            }
        }
        state.invalidate_geometry();
        dialog.visible = false;
        self.hits.clear();
        None
    }
    pub fn preferences_request(&mut self) -> Option<Request> {
        if self.management.preferences_pending.is_some() {
            return None;
        }
        let dialog = self.management.dialog.as_ref()?;
        if dialog.blocked || !self.management_identity(&dialog.target) {
            return None;
        }
        let dialog = self.management.dialog.as_mut()?;
        let state = dialog.preferences.as_mut()?;
        let work = state.requested.take()?;
        self.management.preferences_sequence = self.management.preferences_sequence.wrapping_add(1);
        state.generation = self.management.preferences_sequence;
        let operation = match &work {
            Work::LoadProxy => Operation::LoadProxy,
            Work::SaveProxy(_) => Operation::SaveProxy,
            Work::TestProxy(_) => Operation::TestProxy,
            Work::SaveProvider(_) => Operation::SaveProvider,
            Work::LoadHeaders(_) => Operation::LoadHeaders,
            Work::SaveHeaders(_) => Operation::SaveHeaders,
        };
        let ticket = Ticket {
            target: dialog.target.clone(),
            generation: state.generation,
            operation,
        };
        self.management.preferences_pending = Some(ticket.clone());
        Some(Request { ticket, work })
    }
    pub fn preferences_completed(
        &mut self,
        ticket: Ticket,
        result: Result<Response, RequestFailure>,
    ) {
        if self.management.preferences_pending.as_ref() != Some(&ticket) {
            return;
        }
        self.management.preferences_pending = None;
        if !self.management_identity(&ticket.target) {
            return;
        }
        let write = matches!(
            ticket.operation,
            Operation::SaveProxy | Operation::SaveProvider | Operation::SaveHeaders
        );
        let unknown = matches!(&result, Err(RequestFailure::Unknown(_)))
            || matches!(&result, Err(RequestFailure::Rejected(ClientError::Rejected(e))) if e.code == maka_protocol::OperationErrorCode::CommitOutcomeUnknown);
        if write {
            if let Entity::Connection(row) = &ticket.target.entity
                && (unknown || matches!(&result, Ok(Response::ProviderSaved(CatalogMutationResult::Committed {..}) | Response::HeadersSaved(maka_protocol::configuration::headers::RequestHeadersReplaceResult::Committed {..})))) {self.connections.rows.retain(|current| current.id != row.id || current.revision > row.revision);}
            self.connections.refresh();
            self.models_catalog_changed();
            self.chat.context.refresh();
        }
        let Some(dialog) = self.management.dialog.as_mut().filter(|d| {
            d.target == ticket.target
                && d.preferences
                    .as_ref()
                    .is_some_and(|s| s.generation == ticket.generation)
        }) else {
            return;
        };
        let state = dialog.preferences.as_mut().unwrap();
        match result {
            Ok(Response::HeadersLoaded(
                maka_protocol::configuration::headers::RequestHeadersQueryResult::Found {
                    names,
                    basis,
                },
            )) if ticket.operation == Operation::LoadHeaders => {
                state.body = Body::Headers(headers::Headers::new(basis, names));
                dialog.error = None;
                dialog.blocked = false;
            }
            Ok(Response::HeadersSaved(
                maka_protocol::configuration::headers::RequestHeadersReplaceResult::Committed {
                    names,
                    basis,
                }
                | maka_protocol::configuration::headers::RequestHeadersReplaceResult::Unchanged {
                    names,
                    basis,
                },
            )) if ticket.operation == Operation::SaveHeaders => {
                state.body = Body::Headers(headers::Headers::new(basis, names));
                state.saved = true;
            }
            Ok(Response::Proxy(snapshot, status)) if ticket.operation == Operation::LoadProxy => {
                state.body = Body::Proxy(Box::new(proxy::Proxy::new(snapshot, status)));
                dialog.error = None;
                dialog.blocked = false;
            }
            Ok(Response::ProxySaved(network_update::UpdateResult::Committed {
                revision,
                credential_status,
            })) if ticket.operation == Operation::SaveProxy => {
                if let Body::Proxy(p) = &mut state.body {
                    p.committed(revision, credential_status);
                    state.saved = true;
                }
            }
            Ok(Response::ProxyTested(output)) if ticket.operation == Operation::TestProxy => {
                if let Body::Proxy(p) = &mut state.body {
                    p.test = Some(output);
                }
            }
            Ok(Response::ProviderSaved(CatalogMutationResult::Committed { .. }))
                if ticket.operation == Operation::SaveProvider =>
            {
                self.management.dialog = None;
                self.hits.clear();
                return;
            }
            Ok(_) => {
                dialog.blocked = true;
                dialog.error = Some(
                    if matches!(state.body, Body::Provider(_) | Body::Overlay(_)) {
                        "connection-edit-conflict"
                    } else {
                        "connection-preferences-conflict"
                    },
                );
            }
            Err(_) => {
                if unknown {
                    state.clear_secrets();
                }
                dialog.blocked = unknown;
                dialog.error = Some(if unknown {
                    "session-edit-unknown"
                } else {
                    "connection-preferences-failed"
                });
            }
        }
        state.invalidate_geometry();
        dialog.visible = false;
        self.hits.clear();
    }
    pub(super) fn abandon_preferences(&mut self) -> bool {
        let unknown = self
            .management
            .preferences_pending
            .take()
            .is_some_and(|ticket| {
                matches!(
                    ticket.operation,
                    Operation::SaveProxy | Operation::SaveProvider | Operation::SaveHeaders
                )
            });
        if let Some(state) = self
            .management
            .dialog
            .as_mut()
            .and_then(|d| d.preferences.as_mut())
        {
            state.requested = None;
            state.clear_secrets();
        }
        unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Locale, LocalePreference, i18n::I18n};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        let mut app = App::new(
            "/unused".into(),
            I18n::new(LocalePreference::Explicit(Locale::En), Locale::En),
        );
        app.connection = ConnectionState::Connected {
            root_id: "root".into(),
            epoch: "epoch".into(),
        };
        app
    }
    fn paint(app: &mut App) -> String {
        paint_at(app, 80, 24)
    }
    fn paint_at(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::view::draw(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    fn status() -> CredentialStatus {
        CredentialStatus {
            locator: network_update::locator(),
            state: CredentialState::Configured {
                credential_id: "c7782f6f-04ab-4c1f-8272-5d585570513f".into(),
                revision: 3,
                updated_at: 100,
            },
        }
    }
    fn snapshot() -> RuntimePolicySnapshot {
        let mut policy = maka_protocol::configuration::policy::RuntimePolicy::default();
        policy.network_proxy.enabled = true;
        policy.network_proxy.auth_enabled = true;
        policy.network_proxy.username = "test-user".into();
        RuntimePolicySnapshot {
            revision: 12,
            policy,
        }
    }
    fn loaded(app: &mut App) {
        app.apply(app.network_proxy_action().unwrap());
        let request = app.preferences_request().unwrap();
        app.preferences_completed(request.ticket(), Ok(Response::Proxy(snapshot(), status())));
        paint(app);
    }
    fn replace(editor: &mut Editor, value: &str) {
        editor.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        editor.insert(value);
    }

    #[test]
    fn preferences197_reopened_and_reloaded_forms_reject_old_reads_and_never_replay_unknown_saves()
    {
        let mut app = app();
        let action = app.network_proxy_action().unwrap();
        app.apply(action.clone());
        let first = app.preferences_request().unwrap();
        app.apply(Action::Manage(Manage::Close));
        app.preferences_completed(first.ticket(), Ok(Response::Proxy(snapshot(), status())));
        assert!(app.management.dialog.is_none());
        app.apply(action);
        let second = app.preferences_request().unwrap();
        app.preferences_completed(first.ticket(), Ok(Response::Proxy(snapshot(), status())));
        assert!(matches!(
            app.management
                .dialog
                .as_ref()
                .unwrap()
                .preferences
                .as_ref()
                .unwrap()
                .body,
            Body::LoadingProxy
        ));
        app.preferences_completed(second.ticket(), Ok(Response::Proxy(snapshot(), status())));
        paint(&mut app);
        app.apply(action_for(Command::Secret(SecretAction::Replace)));
        let state = app
            .management
            .dialog
            .as_mut()
            .unwrap()
            .preferences
            .as_mut()
            .unwrap();
        state.editor_mut(3).unwrap().insert("private-proxy-secret");
        let screen = paint(&mut app);
        assert!(!screen.contains("private-proxy-secret"));
        assert!(!app.preferences_enabled(&Command::Test));
        app.apply(action_for(Command::Save));
        let save = app.preferences_request().unwrap();
        assert!(app.preferences_request().is_none());
        let Work::SaveProxy(update) = &save.work else {
            panic!()
        };
        assert_eq!(update.expected_policy_revision, 12);
        assert_eq!(update.expected_credential.as_ref().unwrap().revision, 3);
        app.preferences_completed(
            save.ticket(),
            Err(RequestFailure::Unknown(ClientError::Timeout)),
        );
        paint(&mut app);
        assert!(!app.preferences_enabled(&Command::Save));
        assert!(app.preferences_request().is_none());
        assert_eq!(
            app.management
                .dialog
                .as_ref()
                .unwrap()
                .preferences
                .as_ref()
                .unwrap()
                .editor(3)
                .unwrap()
                .text(),
            ""
        );
        app.apply(action_for(Command::Reload));
        let reload = app.preferences_request().unwrap();
        app.preferences_completed(second.ticket(), Ok(Response::Proxy(snapshot(), status())));
        assert!(matches!(
            app.management
                .dialog
                .as_ref()
                .unwrap()
                .preferences
                .as_ref()
                .unwrap()
                .body,
            Body::LoadingProxy
        ));
        app.connection = ConnectionState::Connected {
            root_id: "other".into(),
            epoch: "epoch".into(),
        };
        app.preferences_completed(reload.ticket(), Ok(Response::Proxy(snapshot(), status())));
        assert!(matches!(
            app.management
                .dialog
                .as_ref()
                .unwrap()
                .preferences
                .as_ref()
                .unwrap()
                .body,
            Body::LoadingProxy
        ));
    }
    fn action_for(command: Command) -> Action {
        Action::Manage(Manage::Preferences(command))
    }

    #[test]
    fn preferences197_proxy_requires_deliberate_secret_choice_for_a_new_account_and_keeps_rejected_edits()
     {
        let mut app = app();
        loaded(&mut app);
        let state = app
            .management
            .dialog
            .as_mut()
            .unwrap()
            .preferences
            .as_mut()
            .unwrap();
        replace(state.editor_mut(0).unwrap(), "other.example");
        let Body::Proxy(proxy) = &state.body else {
            panic!()
        };
        assert_eq!(proxy.update().err(), Some("proxy-target-changed"));
        paint(&mut app);
        assert!(!app.preferences_enabled(&Command::Save));
        app.apply(action_for(Command::Secret(SecretAction::Replace)));
        app.management
            .dialog
            .as_mut()
            .unwrap()
            .preferences
            .as_mut()
            .unwrap()
            .editor_mut(3)
            .unwrap()
            .insert("entered-once");
        paint(&mut app);
        app.apply(action_for(Command::Save));
        let save = app.preferences_request().unwrap();
        app.preferences_completed(
            save.ticket(),
            Ok(Response::ProxySaved(
                network_update::UpdateResult::RevisionConflict {
                    expected_revision: 12,
                    actual_revision: 13,
                },
            )),
        );
        paint(&mut app);
        let state = app
            .management
            .dialog
            .as_ref()
            .unwrap()
            .preferences
            .as_ref()
            .unwrap();
        assert_eq!(state.editor(3).unwrap().text(), "entered-once");
        assert!(!app.preferences_enabled(&Command::Save));
        assert!(app.preferences_enabled(&Command::Reload));
    }

    #[test]
    fn preferences197_recovery_controls_fit_and_remain_keyboard_and_pointer_reachable() {
        use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
        for locale in Locale::ALL {
            for (width, height) in [(80, 24), (40, 18)] {
                for pointer in [false, true] {
                    let mut app = app();
                    app.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
                    loaded(&mut app);
                    app.apply(action_for(Command::Secret(SecretAction::Replace)));
                    app.management
                        .dialog
                        .as_mut()
                        .unwrap()
                        .preferences
                        .as_mut()
                        .unwrap()
                        .editor_mut(3)
                        .unwrap()
                        .insert("private-recovery-secret");
                    paint_at(&mut app, width, height);
                    assert!(app.management.dialog.as_ref().unwrap().visible);
                    app.apply(action_for(Command::Save));
                    let save = app
                        .preferences_request()
                        .expect("a presented form can save");
                    app.preferences_completed(
                        save.ticket(),
                        Ok(Response::ProxySaved(
                            network_update::UpdateResult::RevisionConflict {
                                expected_revision: 12,
                                actual_revision: 13,
                            },
                        )),
                    );
                    let screen = paint_at(&mut app, width, height);
                    assert!(
                        app.management.dialog.as_ref().unwrap().visible,
                        "localized error and recovery still fit at {width}x{height}"
                    );
                    assert!(!screen.contains("private-recovery-secret"));
                    assert!(app.preferences_enabled(&Command::Reload));
                    assert!(!app.preferences_enabled(&Command::Save));
                    assert_eq!(
                        app.layer.focused_path(),
                        Some("footer/cancel"),
                        "error recovery never defaults to discarding edits"
                    );
                    let reload = app
                        .layer
                        .rect("fields/rows/reload")
                        .filter(|rect| !rect.is_empty())
                        .expect("recovery is visible in the body");
                    assert!(app.preferences_request().is_none());
                    if pointer {
                        for kind in [
                            MouseEventKind::Down(MouseButton::Left),
                            MouseEventKind::Up(MouseButton::Left),
                        ] {
                            app.input(Event::Mouse(MouseEvent {
                                kind,
                                column: reload.x,
                                row: reload.y,
                                modifiers: KeyModifiers::NONE,
                            }));
                        }
                    } else {
                        app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
                        paint_at(&mut app, width, height);
                        assert_eq!(app.layer.focused_path(), Some("fields/rows/reload"));
                        app.input(Event::Key(KeyEvent::new(
                            KeyCode::Enter,
                            KeyModifiers::NONE,
                        )));
                    }
                    let reload = app
                        .preferences_request()
                        .expect("explicit visible recovery requests a fresh read");
                    assert!(matches!(reload.work, Work::LoadProxy));
                    assert!(
                        app.preferences_request().is_none(),
                        "recovery never replays the failed write"
                    );
                }
            }
        }
    }

    #[test]
    fn preferences197_short_proxy_diagnostic_keeps_its_secondary_test_action_in_scroll_navigation()
    {
        use crossterm::event::Event;
        let mut app = app();
        loaded(&mut app);
        let Body::Proxy(proxy) = &mut app
            .management
            .dialog
            .as_mut()
            .unwrap()
            .preferences
            .as_mut()
            .unwrap()
            .body
        else {
            panic!()
        };
        proxy.test = Some(network_test::Output::failed(
            "raw error must not be rendered",
        ));
        for _ in 0..24 {
            paint_at(&mut app, 40, 18);
            assert!(app.management.dialog.as_ref().unwrap().visible);
            if app.layer.focused_path() == Some("fields/rows/test") {
                break;
            }
            app.input(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        }
        assert_eq!(app.layer.focused_path(), Some("fields/rows/test"));
        let test = app.layer.rect("fields/rows/test").unwrap();
        assert!(!test.is_empty());
        assert!(
            app.layer
                .rect("footer/cancel")
                .is_some_and(|rect| !rect.is_empty())
        );
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            app.preferences_request().unwrap().work,
            Work::TestProxy(_)
        ));
    }

    #[test]
    fn preferences197_header_validation_does_not_move_fields_or_retarget_the_save_pointer() {
        use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
        use maka_protocol::configuration::headers::{
            RequestHeadersBasis, RequestHeadersQueryResult,
        };
        for locale in Locale::ALL {
            for (width, height) in [(80, 24), (110, 40)] {
                let mut app = app();
                app.i18n = I18n::new(LocalePreference::Explicit(locale), locale);
                let row = std::sync::Arc::new(crate::pages::connections::Row {
                    id: "6f168dd2-0ea4-43b0-925c-38f224ac0589".into(),
                    name: "Fixture".into(),
                    slug: "fixture".into(),
                    provider: crate::providers::fixtures::entry("openai-compatible", false)
                        .identity,
                    configuration: serde_json::json!({}),
                    request_body_overlay: None,
                    enabled: true,
                    enabled_models: 0,
                    model_ids: vec![],
                    revision: 8,
                    default_model: None,
                });
                app.connections.rows.push(row.clone());
                let open = app
                    .connection_management_commands_for(&row)
                    .into_iter()
                    .find_map(|(action, _)| {
                        matches!(
                            &action,
                            Action::Manage(Manage::Open(
                                _,
                                Kind::Connection(connection::Change::RequestHeaders)
                            ))
                        )
                        .then_some(action)
                    })
                    .unwrap();
                app.apply(open);
                let query = app.preferences_request().unwrap();
                app.preferences_completed(
                    query.ticket(),
                    Ok(Response::HeadersLoaded(RequestHeadersQueryResult::Found {
                        names: vec![],
                        basis: RequestHeadersBasis {
                            connection: ConnectionVersionBasis {
                                connection_id: row.id.clone(),
                                revision: row.revision,
                            },
                            credential: None,
                        },
                    })),
                );
                paint_at(&mut app, width, height);
                app.apply(action_for(Command::AddHeader));
                paint_at(&mut app, width, height);
                assert!(
                    !app.preferences_enabled(&Command::Save),
                    "an empty header cannot be saved"
                );
                let save = app.layer.rect("footer/save").unwrap();
                let name = app.layer.rect("fields/rows/0").unwrap();
                let value = app.layer.rect("fields/rows/1").unwrap();
                app.apply(action_for(Command::Field(0)));
                app.input(Event::Paste("X-Trace".into()));
                paint_at(&mut app, width, height);
                app.apply(action_for(Command::Field(1)));
                app.input(Event::Paste("private-header-value".into()));
                let screen = paint_at(&mut app, width, height);
                assert!(!screen.contains("private-header-value"));
                assert!(app.preferences_enabled(&Command::Save));
                assert_eq!(app.layer.rect("fields/rows/0"), Some(name));
                assert_eq!(app.layer.rect("fields/rows/1"), Some(value));
                assert_eq!(
                    app.layer.rect("footer/save"),
                    Some(save),
                    "valid input cannot place Add header under the previous Save position"
                );
                for kind in [
                    MouseEventKind::Down(MouseButton::Left),
                    MouseEventKind::Up(MouseButton::Left),
                ] {
                    app.input(Event::Mouse(MouseEvent {
                        kind,
                        column: save.x,
                        row: save.y,
                        modifiers: KeyModifiers::NONE,
                    }));
                }
                let Work::SaveHeaders(update) = app.preferences_request().unwrap().work else {
                    panic!("Save remains the clicked operation")
                };
                assert_eq!(update.headers.len(), 1);
                assert_eq!(update.headers[0].name, "X-Trace");
            }
        }
    }

    #[test]
    fn preferences197_provider_controls_keep_opaque_configuration_and_validate_scalar_values() {
        let mut descriptor = crate::providers::fixtures::entry("fixture", false).descriptor;
        descriptor.configuration_schema = serde_json::json!({"type":"object", "properties":{"baseUrl":{"type":"string","minLength":1}, "limit":{"type":"integer","minimum":1,"maximum":20},"enabled":{"type":"boolean"},"mode":{"enum":["a","b"]},"nullable":{"enum":["a",null]},"missing":{"enum":["a",null]}},"required":["baseUrl"]});
        let initial = serde_json::json!({"baseUrl":"https://before.example", "limit":3,"enabled":true,"mode":"a","nullable":null,"opaque":{"nested":[1,2]}});
        let mut form = provider::Form::new(&initial, Some(&descriptor));
        assert_eq!(
            form.value().unwrap(),
            initial,
            "explicit null survives while absent remains absent"
        );
        let base = form.fields.iter().position(|f| f.key == "baseUrl").unwrap();
        let limit = form.fields.iter().position(|f| f.key == "limit").unwrap();
        replace(
            form.fields[base].editor.as_mut().unwrap(),
            "https://after.example",
        );
        replace(form.fields[limit].editor.as_mut().unwrap(), "21");
        assert!(form.value().is_err());
        replace(form.fields[limit].editor.as_mut().unwrap(), "9");
        let value = form.value().unwrap();
        assert_eq!(value["limit"], 9);
        assert_eq!(value["opaque"], initial["opaque"]);
        assert_eq!(value["enabled"], true);
        assert!(value.as_object().unwrap().contains_key("nullable"));
        assert!(!value.as_object().unwrap().contains_key("missing"));
        assert!(form.changed());
    }

    #[test]
    fn preferences197_headers_keep_replace_delete_without_reading_or_debugging_secret_values() {
        use maka_protocol::configuration::headers::RequestHeadersBasis;
        let basis = RequestHeadersBasis {
            connection: ConnectionVersionBasis {
                connection_id: "6f168dd2-0ea4-43b0-925c-38f224ac0589".into(),
                revision: 8,
            },
            credential: None,
        };
        let mut headers = headers::Headers::new(
            basis.clone(),
            vec!["X-Keep".into(), "X-Replace".into(), "X-Delete".into()],
        );
        headers.rows[1].action = SecretAction::Replace;
        headers.rows[1].value.insert("private-header-value");
        headers.rows[2].action = SecretAction::Delete;
        let update = headers.update().unwrap();
        assert_eq!(update.expected, basis);
        assert_eq!(update.headers.len(), 2);
        assert!(update.headers[0].value.is_none());
        assert_eq!(
            update.headers[1].value.as_deref(),
            Some("private-header-value")
        );
        replace(&mut headers.rows[0].name, "X-New");
        assert_eq!(headers.update().err(), Some("headers-value-required"));
        headers.clear_secrets();
        assert!(headers.rows[1].value.text().is_empty());
    }
}
