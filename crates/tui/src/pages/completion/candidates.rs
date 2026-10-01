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

use super::{
    io::PAGE,
    model::{Candidate, Context, Cursor, Pick},
};
use crate::app::{Action, App};
use maka_plugins::terminal_ui::Placement;
use maka_protocol::plugin::CommandProjection;

pub(super) fn matches(query: &str, values: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    let query = query.to_lowercase();
    values
        .into_iter()
        .any(|value| value.as_ref().to_lowercase().contains(&query))
}

pub(super) fn commands(
    app: &App,
    context: &Context,
    query: &str,
    offset: usize,
) -> (Vec<Candidate>, Option<Cursor>) {
    let mut native = vec![("help", "command-help", Action::Help)];
    if context.draft.input.is_none() {
        native.insert(0, ("new", "session-create", Action::CreateSession));
        if let Some(action) = app.model_action() {
            native.push(("model", "session-model-change", action));
        }
        native.push((
            "search",
            "chat-search",
            Action::Search(crate::ui::transcript::search::Command::Open),
        ));
        for (alias, commands) in [
            ("branch", app.branch_commands()),
            ("revise", app.revision_commands()),
            ("resume", app.resume_commands()),
        ] {
            for (action, label) in commands {
                native.push((alias, label, action));
            }
        }
    }
    let local = native.into_iter().filter_map(|(name, label, action)| {
        let title = app.i18n.text(label);
        matches(query, [name, title.as_str()]).then(|| Candidate {
            id: format!("native:{name}"),
            title: format!("/{name}  {title}"),
            detail: String::new(),
            source: "Maka".into(),
            enabled: app.enabled(&action),
            pick: Pick::Native(action),
        })
    });
    let locale = app.i18n.locale().id();
    let plugins = app
        .apps
        .directory
        .iter()
        .filter(|entry| entry.descriptor.placement == Placement::Page)
        .filter_map(|entry| {
            crate::apps::Key::of(entry, Some(&context.session)).map(|key| (entry, key))
        })
        .flat_map(|(entry, key)| {
            entry.descriptor.commands.iter().filter_map(move |command| {
                let title = command.title.resolve(locale);
                let found = matches(
                    query,
                    std::iter::once(command.name.as_str())
                        .chain(command.aliases.iter().map(String::as_str))
                        .chain([title]),
                );
                found.then(|| Candidate {
                    id: format!(
                        "command:{}:{}:{}:{}",
                        entry.package_id, entry.method, entry.target.registration, command.name
                    ),
                    title: format!("/{}  {title}", command.name),
                    detail: command.description.resolve(locale).into(),
                    source: entry.package_id.clone(),
                    enabled: app.enabled(&Action::Apps(crate::apps::Message::Open(
                        key.at(command.route.clone()),
                    ))),
                    pick: Pick::Command(CommandProjection {
                        package_id: entry.package_id.clone(),
                        scope_id: entry.scope_id.clone(),
                        method: entry.method.clone(),
                        target: entry.target.clone(),
                        command: command.clone(),
                    }),
                })
            })
        });
    let mut rows: Vec<_> = local.chain(plugins).skip(offset).take(PAGE + 1).collect();
    let next = (rows.len() > PAGE).then_some(Cursor::Local(offset + PAGE));
    rows.truncate(PAGE);
    (rows, next)
}

pub(super) fn command_action(
    app: &App,
    context: &Context,
    command: &CommandProjection,
) -> Option<Action> {
    let entry = app.apps.directory.iter().find(|entry| {
        entry.package_id == command.package_id
            && entry.method == command.method
            && entry.scope_id == command.scope_id
            && entry.target == command.target
            && entry.descriptor.placement == Placement::Page
            && entry
                .descriptor
                .commands
                .iter()
                .any(|current| current == &command.command)
    })?;
    let key =
        crate::apps::Key::of(entry, Some(&context.session))?.at(command.command.route.clone());
    let action = Action::Apps(crate::apps::Message::Open(key));
    app.enabled(&action).then_some(action)
}
