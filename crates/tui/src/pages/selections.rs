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

//! Saved input selectors are user drafts, independent of the provider lifecycle.
use crate::{
    app::{Action, App},
    pages::references::Target,
    ui::{Node, On, Role},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picked {
    // Compatibility with the previous domain-specific draft format only.
    #[serde(default = "legacy_provider")]
    pub provider: String,
    pub id: String,
    pub name: String,
}
fn legacy_provider() -> String {
    "maka.skills".into()
}
pub fn selections(items: &[Picked]) -> maka_runtime::input::Selections {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    for item in items {
        result
            .entry(item.provider.clone())
            .or_default()
            .push(item.id.clone());
    }
    result
}
pub fn validate(items: &[Picked]) -> Result<(), String> {
    let mut seen = HashSet::new();
    if items.iter().any(|item| {
        !seen.insert((&item.provider, &item.id))
            || item.name.len() > 4096
            || item.name.chars().any(char::is_control)
    }) {
        return Err("Invalid saved input selection".into());
    }
    maka_runtime::input::validate_selections(&selections(items)).map_err(str::to_owned)
}
#[derive(Default)]
pub struct State {
    pub saved: BTreeMap<String, Vec<Picked>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub target: Target,
    pub provider: String,
    pub id: String,
}
impl Command {
    pub fn label(&self) -> &'static str {
        "completion-remove"
    }
}
impl App {
    pub(crate) fn has_selections(&self, session: &str) -> bool {
        self.selections
            .saved
            .get(session)
            .is_some_and(|items| !items.is_empty())
    }
    pub(crate) fn picked_selections(&self, target: &Target) -> &[Picked] {
        match &target.input {
            Some(input) => self
                .revision
                .selections(&target.session, input)
                .unwrap_or_default(),
            None => self
                .selections
                .saved
                .get(&target.session)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        }
    }
    pub(crate) fn selections_enabled(&self, command: &Command) -> bool {
        self.reference_target().as_ref() == Some(&command.target)
            && self.reference_editable(&command.target)
            && self
                .picked_selections(&command.target)
                .iter()
                .any(|item| item.provider == command.provider && item.id == command.id)
    }
    pub(crate) fn selections_action(&mut self, command: Command) {
        if !self.selections_enabled(&command) {
            return;
        }
        let items = match &command.target.input {
            Some(input) => self.revision.selections_mut(&command.target.session, input),
            None => self.selections.saved.get_mut(&command.target.session),
        };
        if let Some(items) = items {
            items.retain(|item| item.provider != command.provider || item.id != command.id);
        }
    }
}
pub fn chips(app: &App, session: &str) -> Node<Action> {
    let Some(target) = app
        .reference_target()
        .filter(|target| target.session == session)
    else {
        return Node::column("selections", vec![]);
    };
    Node::row(
        "selections",
        app.picked_selections(&target)
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let command = Command {
                    target: target.clone(),
                    provider: item.provider.clone(),
                    id: item.id.clone(),
                };
                let enabled = app.selections_enabled(&command);
                Node::button(
                    index.to_string(),
                    format!("{} ×", crate::view::safe(&item.name)),
                    Role::Normal,
                )
                .on(On::Activate(Action::Selections(command)))
                .enabled(enabled)
            })
            .collect(),
    )
}
