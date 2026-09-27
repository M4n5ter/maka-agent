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

pub(crate) struct ReviewItem {
    pub target: SelectionTarget,
    pub label: String,
    pub quote: bool,
}

impl State {
    pub(crate) fn completion_reviewing(&self) -> bool {
        self.visible
            && self.phase == Phase::Editing
            && self.pending.is_none()
            && self.requested.is_none()
            && self
                .saved
                .as_ref()
                .is_some_and(|saved| saved.stage == crate::pages::revision::saved::Stage::Bindings)
    }
    pub(crate) fn completion_focus(&mut self, key: &DraftKey) -> bool {
        let Some(index) = self.completion_index(key) else {
            return false;
        };
        self.switch_editor(index, key.display);
        true
    }
    pub(crate) fn completion_has_mark(&self, key: &DraftKey, id: &str) -> bool {
        if let Some(editor) = self.completion_editor(key) {
            return editor.marks().iter().any(|mark| mark.id == id);
        }
        let Some(index) = self.completion_index(key) else {
            return false;
        };
        let input = &self.saved.as_ref().unwrap().inputs[index];
        (if key.display {
            &input.display_marks
        } else {
            &input.marks
        })
        .iter()
        .any(|mark| mark.id == id)
    }
    pub(crate) fn completion_original_selected(
        &self,
        key: &DraftKey,
        provider: &str,
        selector: &str,
    ) -> bool {
        let Some(index) = self.completion_index(key) else {
            return false;
        };
        let input = &self.saved.as_ref().unwrap().inputs[index];
        input
            .original
            .input_selection_sources
            .iter()
            .any(|source| source.provider == provider)
            && input
                .original
                .input_selections
                .get(provider)
                .and_then(|items| items.iter().position(|id| id == selector))
                .is_some_and(|index| {
                    input.included(&crate::pages::revision::resources::Resource::Selection {
                        provider: provider.into(),
                        index,
                    })
                })
    }
    pub(crate) fn completion_resolve_original(
        &mut self,
        key: &DraftKey,
        provider: &str,
        selector: &str,
        binding: Binding,
    ) -> bool {
        if !self.completion_original_selected(key, provider, selector) {
            return false;
        }
        let index = self.completion_index(key).unwrap();
        let input = &mut self.saved.as_mut().unwrap().inputs[index];
        input.resolved.retain(|item| !matches!(&item.payload, Payload::Selection { provider: p, selector: s, .. } if p == provider && s == selector));
        input.resolved.push(binding);
        self.error = None;
        true
    }
    pub(crate) fn completion_drop_original(
        &mut self,
        key: &DraftKey,
        provider: &str,
        selector: &str,
    ) {
        if !self.completion_original_selected(key, provider, selector) {
            return;
        }
        let input_index = self.completion_index(key).unwrap();
        let input = &mut self.saved.as_mut().unwrap().inputs[input_index];
        let index = input.original.input_selections[provider]
            .iter()
            .position(|id| id == selector)
            .unwrap();
        input.toggle(&crate::pages::revision::resources::Resource::Selection {
            provider: provider.into(),
            index,
        });
        input.prune_resolved();
        self.error = None;
    }
    pub(in crate::pages::revision) fn completion_review_items(&self) -> Vec<ReviewItem> {
        let Some(saved) = &self.saved else {
            return vec![];
        };
        if saved.stage != crate::pages::revision::saved::Stage::Bindings {
            return vec![];
        }
        let Some(input) = saved.inputs.get(self.selected) else {
            return vec![];
        };
        let session = &saved.copy.target_session_id;
        let message = input.message();
        let mut first = BTreeMap::new();
        let mut conflicts = HashSet::new();
        for source in &message.input_selection_sources {
            if first
                .insert(&source.provider, source)
                .is_some_and(|old| old != source)
            {
                conflicts.insert(source.provider.as_str());
            }
        }
        let mut rows = vec![];
        for display in [false, true] {
            for mark in if display {
                &input.display_marks
            } else {
                &input.marks
            } {
                let Some(binding) = input.bindings.get(&mark.id) else {
                    continue;
                };
                let Payload::Selection {
                    provider,
                    source: Some(source),
                    quote,
                    ..
                } = &binding.payload
                else {
                    continue;
                };
                if source.session_id == *session && !conflicts.contains(provider.as_str()) {
                    continue;
                }
                rows.push(ReviewItem {
                    target: SelectionTarget {
                        draft: DraftKey {
                            session: session.clone(),
                            input: Some(input.original.message_id.clone()),
                            display,
                        },
                        key: ReferenceKey::Mark(mark.id.clone()),
                    },
                    label: binding.label.clone(),
                    quote: quote.is_some(),
                });
            }
        }
        for (provider, selectors) in &input.original.input_selections {
            let Some(original) = input
                .original
                .input_selection_sources
                .iter()
                .find(|source| source.provider == *provider)
            else {
                continue;
            };
            for (index, selector) in selectors.iter().enumerate() {
                if !input.included(&crate::pages::revision::resources::Resource::Selection {
                    provider: provider.clone(),
                    index,
                }) {
                    continue;
                }
                let source = input
                    .resolved
                    .iter()
                    .find_map(|binding| match &binding.payload {
                        Payload::Selection {
                            provider: p,
                            selector: s,
                            source: Some(source),
                            ..
                        } if p == provider && s == selector => Some(source),
                        _ => None,
                    })
                    .unwrap_or(original);
                if source.session_id == *session && !conflicts.contains(provider.as_str()) {
                    continue;
                }
                rows.push(ReviewItem {
                    target: SelectionTarget {
                        draft: DraftKey {
                            session: session.clone(),
                            input: Some(input.original.message_id.clone()),
                            display: self.display,
                        },
                        key: ReferenceKey::Original {
                            provider: provider.clone(),
                            selector: selector.clone(),
                        },
                    },
                    label: format!("{provider} · {selector}"),
                    quote: false,
                });
            }
        }
        rows
    }
}
impl Input {
    pub(in crate::pages::revision) fn needs_resource_review(&self, session: &str) -> bool {
        let message = self.message();
        message
            .input_selection_sources
            .iter()
            .any(|source| source.session_id != session)
            || maka_runtime::input::validate_selection_sources(
                &message.input_selections,
                &message.input_selection_sources,
            )
            .is_err()
    }
}

pub(in crate::pages::revision) fn review_rows(
    app: &crate::app::App,
    sheet: crate::ui::Sheet<crate::app::Action>,
    height: u16,
) -> crate::ui::Sheet<crate::app::Action> {
    use crate::{
        app::Action,
        pages::completion::Command,
        ui::{Node, On, Role, Size, Tone},
    };
    let rows = app
        .revision
        .completion_review_items()
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let button = |key: &str, command: Command| {
                Node::button(key.to_owned(), app.i18n.text(command.label()), Role::Normal)
                    .enabled(app.completion_enabled(&command))
                    .on(On::Activate(Action::Completion(command)))
            };
            let mut actions = vec![
                button("reselect", Command::Reselect(item.target.clone())),
                button("remove", Command::DropSelection(item.target.clone())),
            ];
            if item.quote {
                actions.push(button("excerpt", Command::KeepExcerpt(item.target)));
            }
            Node::column(
                index.to_string(),
                vec![
                    Node::text(
                        "label",
                        vec![(crate::view::safe(&item.label), Tone::Warning)],
                    )
                    .clip(),
                    Node::row("actions", actions).gap(1),
                ],
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return sheet;
    }
    sheet.body(
        Node::scroll("reference-review", Node::column("items", rows))
            .size(Size::Upto(height.saturating_sub(20).max(4))),
    )
}
