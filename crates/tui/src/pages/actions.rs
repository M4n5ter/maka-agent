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

//! Native object menus: the same domain actions used by ordinary dialogs.
pub(crate) mod native;
use crate::{
    app::{Action, App, ConnectionState},
    navigation::Route,
    ui::{Align, MenuItem, Node, On, Role, Size, Tone},
};

pub(crate) fn identity(app: &App, object: &str) -> String {
    match &app.connection {
        ConnectionState::Connected { root_id, epoch } => format!("{root_id}/{epoch}/{object}"),
        _ => format!("disconnected/{object}"),
    }
}

pub(crate) fn session_scope(app: &App, id: &str) -> String {
    // Explicit selection changes the object. Background scrolling does not;
    // message-copy actions carry the exact key that was visible when opened.
    let selected = app
        .chat
        .reader()
        .and_then(|reader| reader.selected().map(|block| block.key));
    let selected = serde_json::to_string(&selected).expect("message keys serialize");
    format!("session/{id}/message/{selected}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyTarget {
    root: String,
    epoch: Option<String>,
    session: String,
    message: crate::ui::transcript::MessageKey,
}

impl CopyTarget {
    fn epoch(app: &App) -> Option<String> {
        match &app.connection {
            ConnectionState::Connected { epoch, .. } => Some(epoch.clone()),
            _ => None,
        }
    }
    fn capture(app: &App) -> Option<Self> {
        let Route::Session(session) = app.navigation.current() else {
            return None;
        };
        Some(Self {
            root: app.checkpoint_root().into(),
            epoch: Self::epoch(app),
            session,
            message: app.chat.reader()?.selection()?,
        })
    }
    fn message(app: &App, message: crate::ui::transcript::MessageKey) -> Option<Self> {
        let Route::Session(session) = app.navigation.current() else {
            return None;
        };
        Some(Self {
            root: app.checkpoint_root().into(),
            epoch: Self::epoch(app),
            session,
            message,
        })
    }
    pub(crate) fn current(&self, app: &App) -> bool {
        self.root == app.checkpoint_root()
            && self.epoch == Self::epoch(app)
            && app.navigation.current() == Route::Session(self.session.clone())
            && !app.chrome.details
    }
    pub(crate) fn text(
        &self,
        app: &App,
        mode: crate::ui::transcript::selection::CopyMode,
    ) -> Result<String, &'static str> {
        if !self.current(app) {
            return Err("chat-copy-empty");
        }
        app.chat.reader().ok_or("chat-copy-empty")?.copy_text_at(
            &self.message,
            mode,
            app.chrome.ascii,
        )
    }
}

pub(crate) fn items(
    app: &App,
    commands: impl IntoIterator<Item = (Action, &'static str)>,
) -> Vec<MenuItem<Action>> {
    let mut seen = std::collections::HashSet::new();
    commands
        .into_iter()
        .filter_map(|(action, label)| {
            // A semantic operation within the explicitly bound object. The surface
            // freezes the Action itself; its label is never used as domain authority.
            let key = if let Action::Apps(crate::apps::Message::Open(key)) = &action {
                use sha2::{Digest, Sha256};
                format!(
                    "app-{:x}",
                    Sha256::digest(serde_json::to_vec(key).expect("View address is serializable"))
                )
            } else {
                label.to_owned()
            };
            if !seen.insert(key.clone()) {
                return None;
            }
            let role = match &action {
                Action::Manage(super::manage::Command::Open(
                    _,
                    super::manage::Kind::Remove
                    | super::manage::Kind::Archive
                    | super::manage::Kind::Connection(super::manage::connection::Change::Remove)
                    | super::manage::Kind::Credential(super::manage::credentials::Change::Clear),
                )) => Role::Destructive,
                _ => Role::Normal,
            };
            Some(MenuItem {
                key,
                label: if matches!(action, Action::Apps(_)) {
                    crate::view::action_label(app, &action)
                } else {
                    app.i18n.text(label)
                },
                enabled: app.enabled(&action),
                action,
                role,
            })
        })
        .collect()
}

pub(crate) fn context(
    app: &App,
    object: &str,
    commands: impl IntoIterator<Item = (Action, &'static str)>,
) -> crate::ui::Menu<Action> {
    crate::ui::Menu {
        identity: identity(app, object),
        items: items(app, commands),
    }
}

pub(crate) fn menu(
    app: &App,
    key: impl Into<std::borrow::Cow<'static, str>>,
    object: &str,
    label: &'static str,
    commands: impl IntoIterator<Item = (Action, &'static str)>,
) -> Node<Action> {
    let items = items(app, commands);
    let enabled = !items.is_empty();
    Node::text(
        key,
        vec![(app.chrome.symbol("⋯", "...").into(), Tone::Muted)],
    )
    .align(Align::Center)
    .clip()
    .size(Size::Fixed(3))
    .on(On::Menu {
        identity: identity(app, object),
        items,
    })
    .enabled(enabled)
    .hint(app.i18n.text(label))
}

pub(crate) fn catalog_session_commands(
    app: &App,
    item: &maka_protocol::session::SessionCatalogProjection,
) -> Vec<(Action, &'static str)> {
    let mut actions = vec![(
        Action::Visit(Route::Session(item.id.clone())),
        "session-open",
    )];
    actions.extend(app.session_management_commands_for(item));
    if app.tabs.contains(&item.id) {
        actions.push((Action::CloseTab(item.id.clone()), "tabs-close"));
    }
    actions
}

pub(crate) fn session_commands(app: &App) -> Vec<(Action, &'static str)> {
    let Route::Session(id) = app.navigation.current() else {
        return vec![];
    };
    let mut commands = vec![(
        Action::Search(crate::ui::transcript::search::Command::Open),
        "chat-search",
    )];
    if !app.pending_new.contains_key(&id) {
        commands.push((Action::RefreshSession, "command-refresh"));
    }
    commands.extend(app.management_commands());
    commands.extend(app.branch_commands());
    commands.extend(app.side_branch_commands());
    commands.extend(app.session_control_commands());
    if let Some(action) = app.resources_action() {
        commands.push((action, "resources-title"));
    }
    commands.extend(app.revision_commands());
    commands.extend(app.resume_commands());
    commands.extend(app.bundle_commands());
    for mode in [
        crate::ui::transcript::selection::CopyMode::Selection,
        crate::ui::transcript::selection::CopyMode::Message,
        crate::ui::transcript::selection::CopyMode::Source,
    ] {
        let action = if mode == crate::ui::transcript::selection::CopyMode::Selection {
            Action::Copy(mode)
        } else if let Some(target) = CopyTarget::capture(app) {
            Action::CopyMessage { target, mode }
        } else {
            continue;
        };
        commands.push((action, mode.label()));
    }
    if app.enabled(&Action::RetrySubmission) {
        commands.push((Action::RetrySubmission, "chat-retry-original"));
    }
    commands.push((
        Action::ToggleTrace,
        if app.chat.view.trace {
            "command-trace-hide"
        } else {
            "command-trace-show"
        },
    ));
    commands.extend(app.header_actions().into_iter().map(|action| {
        let label = match action {
            Action::ToggleDetails => "command-details",
            Action::ToggleFullscreen => "command-fullscreen",
            Action::ToggleInspector if app.navigation.location().inspector => {
                "command-inspector-hide"
            }
            Action::ToggleInspector => "command-inspector-show",
            Action::OlderMessages => "chat-older",
            Action::NewerMessages => "chat-newer",
            Action::LatestMessages => "chat-latest",
            Action::OpenInteraction => "interaction-open",
            Action::Inbox => "command-inbox",
            _ => unreachable!("unexpected session header action"),
        };
        (action, label)
    }));
    commands.push((Action::CloseTab(id), "tabs-close"));
    commands
}

pub(crate) fn connection_commands(
    app: &App,
    row: &std::sync::Arc<super::connections::Row>,
) -> Vec<(Action, &'static str)> {
    let mut commands = app.connection_management_commands_for(row);
    commands.extend(app.connection_reauthentication_command_for(row));
    commands
}

pub(crate) fn project_commands(
    app: &App,
    project: &super::projects::Item,
) -> Vec<(Action, &'static str)> {
    let mut commands = vec![(
        Action::Project(super::projects::Command::Create(project.id.clone())),
        "project-create-session",
    )];
    if project.usable() {
        commands.extend(
            app.workspace_launches(maka_runtime::execution::WorkspaceTarget::Project {
                project_id: project.id.clone(),
            })
            .into_iter()
            .map(|action| (action, "route-extensions")),
        );
    }
    commands.extend(app.project_management_commands_for(project));
    commands
}

pub(crate) fn composer_add() -> Vec<(Action, &'static str)> {
    vec![
        (
            Action::Attachment(super::attachments::Command::Open),
            "attachments-add",
        ),
        (Action::References, "references-title"),
        (
            Action::Completion(super::completion::Command::Resources),
            "completion-plugins",
        ),
        (
            Action::Completion(super::completion::Command::Open(
                crate::editor::completion::Kind::Reference,
            )),
            "completion-context",
        ),
        (
            Action::Completion(super::completion::Command::Open(
                crate::editor::completion::Kind::Command,
            )),
            "completion-commands",
        ),
        (
            Action::Attachment(super::attachments::Command::Paste),
            "composer-paste",
        ),
    ]
}

#[cfg(test)]
mod tests;
