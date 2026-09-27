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

use super::{Phase, State, draft::Input, saved::Checkpoint};
use crate::{
    editor::{Editor, marks::Mark, saved::Saved},
    pages::completion::{Binding, DraftKey, Payload, ReferenceKey, SelectionTarget, bindings},
};
use std::collections::{BTreeMap, HashSet};

impl State {
    fn completion_index(&self, key: &DraftKey) -> Option<usize> {
        let saved = self
            .saved
            .as_ref()
            .filter(|saved| saved.copy.target_session_id == key.session)?;
        saved
            .inputs
            .iter()
            .position(|input| Some(&input.original.message_id) == key.input.as_ref())
    }
    pub(crate) fn completion_target(&self) -> Option<DraftKey> {
        if !self.visible
            || self.phase != Phase::Editing
            || self.confirm_discard
            || self.resources.visible
            || self.show_problem
            || self.pending.is_some()
            || self.requested.is_some()
        {
            return None;
        }
        let target = self.directory_target()?;
        Some(DraftKey {
            session: target.session,
            input: target.input,
            display: self.display,
        })
    }
    pub(crate) fn completion_editor(&self, key: &DraftKey) -> Option<&Editor> {
        let index = self.completion_index(key)?;
        if index == self.selected && key.display == self.display {
            Some(&self.editor)
        } else {
            self.editors
                .iter()
                .find(|(position, _)| *position == (index, key.display))
                .map(|(_, editor)| editor)
        }
    }
    pub(crate) fn completion_editor_mut(&mut self, key: &DraftKey) -> Option<&mut Editor> {
        let index = self.completion_index(key)?;
        if index == self.selected && key.display == self.display {
            Some(&mut self.editor)
        } else {
            self.editors
                .iter_mut()
                .find(|(position, _)| *position == (index, key.display))
                .map(|(_, editor)| editor)
        }
    }
    pub(crate) fn completion_bindings(&self, key: &DraftKey) -> Option<&BTreeMap<String, Binding>> {
        let index = self.completion_index(key)?;
        Some(&self.saved.as_ref()?.inputs[index].bindings)
    }
    pub(crate) fn completion_bindings_mut(
        &mut self,
        key: &DraftKey,
    ) -> Option<&mut BTreeMap<String, Binding>> {
        let index = self.completion_index(key)?;
        Some(&mut self.saved.as_mut()?.inputs[index].bindings)
    }
    pub(crate) fn completion_commit(&mut self, key: &DraftKey) -> Result<(), &'static str> {
        let index = self.completion_index(key).ok_or("completion-stale")?;
        let editor = self.completion_editor(key).ok_or("completion-stale")?;
        let text = editor.text().to_owned();
        let marks = editor.marks().to_vec();
        let input = &mut self.saved.as_mut().unwrap().inputs[index];
        input.replace(text, key.display)?;
        if key.display {
            input.display_marks = marks;
        } else {
            input.marks = marks;
        }
        Ok(())
    }
    pub(crate) fn completion_trim_history(&mut self) {
        self.editor.clear_history();
        for (_, editor) in &mut self.editors {
            editor.clear_history();
        }
    }
    pub(crate) fn completion_bytes(&self) -> usize {
        self.saved.as_ref().map_or(0, |saved| {
            saved
                .inputs
                .iter()
                .map(|input| {
                    input
                        .bindings
                        .iter()
                        .map(|(id, binding)| id.len() + bindings::binding_bytes(binding))
                        .sum::<usize>()
                        + input
                            .resolved
                            .iter()
                            .map(bindings::binding_bytes)
                            .sum::<usize>()
                })
                .sum()
        })
    }
    pub(crate) fn completion_gc(&mut self) {
        let Some(saved) = &self.saved else {
            return;
        };
        let mut retained: Vec<HashSet<String>> = saved
            .inputs
            .iter()
            .map(|input| {
                input
                    .marks
                    .iter()
                    .chain(&input.display_marks)
                    .map(|mark| mark.id.clone())
                    .collect()
            })
            .collect();
        if let Some(ids) = retained.get_mut(self.selected) {
            ids.extend(self.editor.retained_mark_ids().map(str::to_owned));
        }
        for ((index, _), editor) in &self.editors {
            if let Some(ids) = retained.get_mut(*index) {
                ids.extend(editor.retained_mark_ids().map(str::to_owned));
            }
        }
        for (input, ids) in self.saved.as_mut().unwrap().inputs.iter_mut().zip(retained) {
            input.bindings.retain(|id, _| ids.contains(id));
            input.prune_resolved();
        }
    }
    pub(super) fn capture_completion(&self, saved: &mut Checkpoint) {
        for ((index, display), editor) in &self.editors {
            if let Some(input) = saved.inputs.get_mut(*index) {
                *input.marks_mut(*display) = editor.marks().to_vec();
            }
        }
        if let Some(input) = saved.inputs.get_mut(self.selected) {
            *input.marks_mut(self.display) = self.editor.marks().to_vec();
        }
        for input in &mut saved.inputs {
            let live: HashSet<_> = input
                .marks
                .iter()
                .chain(&input.display_marks)
                .map(|mark| &mark.id)
                .collect();
            input.bindings.retain(|id, _| live.contains(id));
            input.prune_resolved();
        }
    }
}

impl Input {
    fn prune_resolved(&mut self) {
        let included: HashSet<_> = self
            .original
            .input_selections
            .iter()
            .flat_map(|(provider, selectors)| {
                selectors
                    .iter()
                    .enumerate()
                    .filter_map(|(index, selector)| {
                        self.included(&super::resources::Resource::Selection {
                            provider: provider.clone(),
                            index,
                        })
                        .then_some((provider.clone(), selector.clone()))
                    })
            })
            .collect();
        self.resolved.retain(|binding| matches!(&binding.payload, Payload::Selection { provider, selector, .. } if included.contains(&(provider.clone(), selector.clone()))));
    }
    pub(super) fn marks_mut(&mut self, display: bool) -> &mut Vec<Mark> {
        if display {
            &mut self.display_marks
        } else {
            &mut self.marks
        }
    }
    pub(super) fn marked_editor(&self, display: bool) -> Result<Editor, &'static str> {
        let text = if display {
            self.content
                .display_text
                .as_deref()
                .ok_or("completion-input-invalid")?
        } else {
            &self.content.text
        };
        Editor::restore(Saved {
            text: text.into(),
            cursor: 0,
            anchor: None,
            upstream: false,
            marks: if display {
                self.display_marks.clone()
            } else {
                self.marks.clone()
            },
        })
    }
    pub(super) fn validate_completion(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        for display in [false, true] {
            let marks = if display {
                &self.display_marks
            } else {
                &self.marks
            };
            if marks.is_empty() {
                continue;
            }
            let editor = self.marked_editor(display)?;
            for mark in editor.marks() {
                if !ids.insert(mark.id.clone()) || !self.bindings.contains_key(&mark.id) {
                    return Err("Invalid revision binding".into());
                }
            }
        }
        let mut selectors = HashSet::new();
        for binding in &self.resolved {
            let Payload::Selection {
                provider,
                selector,
                source: Some(_),
                ..
            } = &binding.payload
            else {
                return Err("Invalid resolved revision reference".into());
            };
            if !selectors.insert((provider, selector))
                || !self
                    .original
                    .input_selection_sources
                    .iter()
                    .any(|source| source.provider == *provider)
                || !self
                    .original
                    .input_selections
                    .get(provider)
                    .is_some_and(|values| values.contains(selector))
            {
                return Err("Unknown resolved revision reference".into());
            }
        }
        Ok(())
    }
}

mod review;
pub(super) use review::review_rows;

#[cfg(test)]
mod tests;
