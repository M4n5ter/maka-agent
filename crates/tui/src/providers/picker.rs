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
        choices: Vec<(String, String, Action)>,
        back: Action,
        enabled: bool,
    ) -> Sheet<Action> {
        self.query.model(
            vec!["providers".into()],
            choices
                .iter()
                .enumerate()
                .map(|(index, (label, identity, _))| Entry {
                    key: index.to_string(),
                    group: "providers".into(),
                    title: label.clone(),
                    summary: identity.clone(),
                })
                .collect(),
            None,
        );
        let mut rows: Vec<_> = self
            .query
            .visible()
            .iter()
            .filter_map(|entry| {
                let index = entry.key.parse::<usize>().ok()?;
                let (label, _, action) = choices.get(index)?;
                Some(
                    Node::text(entry.key.clone(), vec![(safe(label), Tone::Normal)])
                        .clip()
                        .on(On::Activate(action.clone()))
                        .enabled(enabled && app.enabled(action)),
                )
            })
            .collect();
        if rows.is_empty() {
            rows.push(Node::text(
                "empty",
                vec![(app.i18n.text("providers-no-matches"), Tone::Subtle)],
            ));
        }
        let height = app.frame_size.map_or(24, |(_, height)| height);
        Sheet::new(key, app.i18n.text("onboard-provider"))
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
                app.i18n.text("onboard-back"),
                Role::Normal,
                back.clone(),
                true,
            )
            .back(back)
            .focus_node("filter")
    }
}
