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

use crate::{
    app::{Action, App},
    ui::{
        Node, On, Role, Sheet, Size, Tone,
        collection::{Collection, Control, Entry},
    },
    view::safe,
};

pub(crate) struct Choice {
    pub label: String,
    pub identity: String,
    pub group: Option<(u8, String)>,
    pub action: Action,
}

/// A local search step over the form's frozen provider inventory.
#[derive(Default)]
pub(crate) struct Picker {
    pub open: bool,
    query: Collection,
}

impl Picker {
    pub fn open(&mut self) {
        self.query = Collection::default();
        self.open = true;
    }

    pub fn offers(&self, index: usize) -> bool {
        self.open
            && self
                .query
                .visible()
                .iter()
                .any(|entry| entry.key == index.to_string())
    }

    pub fn sheet(
        &self,
        app: &App,
        key: String,
        choices: Vec<Choice>,
        title: String,
        back: Action,
        enabled: bool,
    ) -> Sheet<Action> {
        let mut groups = Vec::new();
        for choice in &choices {
            let group = choice.group.clone().unwrap_or((0, "providers".into()));
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        groups.sort_by_key(|(order, _)| *order);
        let groups: Vec<_> = groups.into_iter().map(|(_, name)| name).collect();
        self.query.model(
            groups.clone(),
            choices
                .iter()
                .enumerate()
                .map(|(index, choice)| Entry {
                    key: index.to_string(),
                    group: choice
                        .group
                        .as_ref()
                        .map_or_else(|| "providers".into(), |(_, name)| name.clone()),
                    title: choice.label.clone(),
                    summary: choice.identity.clone(),
                })
                .collect(),
            None,
        );
        let visible = self.query.visible();
        let mut rows = Vec::new();
        for (section, group) in groups.iter().enumerate() {
            let entries: Vec<_> = visible
                .iter()
                .filter(|entry| &entry.group == group)
                .collect();
            if entries.is_empty() {
                continue;
            }
            if group != "providers" {
                rows.push(Node::text(
                    format!("section-{section}"),
                    vec![(group.clone(), Tone::Muted)],
                ));
            }
            for entry in entries {
                let index = entry.key.parse::<usize>().expect("local picker index");
                let choice = &choices[index];
                rows.push(
                    Node::text(entry.key.clone(), vec![(safe(&choice.label), Tone::Normal)])
                        .clip()
                        .on(On::Activate(choice.action.clone()))
                        .enabled(enabled && app.enabled(&choice.action)),
                );
            }
        }
        if rows.is_empty() {
            rows.push(Node::text(
                "empty",
                vec![(app.i18n.text("providers-no-matches"), Tone::Subtle)],
            ));
        }
        let height = app.frame_size.map_or(24, |(_, height)| height);
        Sheet::new(key, title)
            .body(
                Node::slot("filter", 1)
                    .on(On::Collection(Control::Query {
                        state: self.query.clone(),
                        label: app.i18n.text("providers-search"),
                        placeholder: app.i18n.text("providers-search-hint"),
                    }))
                    .enabled(enabled),
            )
            .body(
                Node::scroll("providers", Node::column("rows", rows).focus_group())
                    .size(Size::Upto(height.saturating_sub(12).max(3))),
            )
            .button(
                "back",
                crate::view::action_label(app, &back),
                Role::Normal,
                back.clone(),
                true,
            )
            .back(back)
            .focus_node("filter")
    }
}
