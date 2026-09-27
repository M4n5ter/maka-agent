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

use super::model::{Payload, ReferenceKey, Reselection, SelectionTarget};
use super::*;

impl App {
    fn completion_selection(&self, target: &SelectionTarget) -> Option<(String, String)> {
        match &target.key {
            ReferenceKey::Mark(id) => {
                let binding = self.completion_bindings(&target.draft)?.get(id)?;
                let Payload::Selection {
                    provider,
                    selector,
                    source: Some(_),
                    ..
                } = &binding.payload
                else {
                    return None;
                };
                (if target.draft.input.is_some() {
                    self.revision.completion_has_mark(&target.draft, id)
                } else {
                    self.completion_editor(&target.draft)
                        .is_some_and(|editor| editor.marks().iter().any(|mark| mark.id == *id))
                })
                .then(|| (provider.clone(), selector.clone()))
            }
            ReferenceKey::Original { provider, selector } => self
                .revision
                .completion_original_selected(&target.draft, provider, selector)
                .then(|| (provider.clone(), selector.clone())),
        }
    }
    pub(super) fn completion_reselection_enabled(
        &self,
        target: &SelectionTarget,
        command: &Command,
    ) -> bool {
        if self.completion_selection(target).is_none()
            || self.completion_draft().is_none_or(|draft| {
                draft.session != target.draft.session || draft.input != target.draft.input
            })
        {
            return false;
        }
        if target.draft.input.is_some() && !self.revision.completion_reviewing() {
            return false;
        }
        if matches!(command, Command::KeepExcerpt(_)) {
            return match &target.key {
                ReferenceKey::Mark(id) => self
                    .completion_bindings(&target.draft)
                    .and_then(|items| items.get(id))
                    .is_some_and(|binding| {
                        matches!(&binding.payload, Payload::Selection { quote: Some(_), .. })
                    }),
                ReferenceKey::Original { .. } => false,
            };
        }
        true
    }
    pub(super) fn completion_reselect(&mut self, target: SelectionTarget) {
        let Some((provider, selector)) = self.completion_selection(&target) else {
            return;
        };
        if target.draft.input.is_some() && !self.revision.completion_focus(&target.draft) {
            return;
        }
        let Some(context) = self.completion_context(Some(Kind::Reference)) else {
            return;
        };
        self.completion_start(context, Source::Providers, true);
        self.completion.reselect = Some(Reselection {
            target,
            provider,
            selector,
        });
    }
    pub(super) fn completion_accept_reselection(&mut self, binding: Binding) {
        let Some(reselect) = self.completion.reselect.clone() else {
            return;
        };
        let Some(popup) = &self.completion.popup else {
            return;
        };
        let context = popup.context.clone();
        if !self.completion_current(&context, popup.explicit)
            || self.completion_selection(&reselect.target).as_ref()
                != Some(&(reselect.provider.clone(), reselect.selector.clone()))
        {
            self.completion_error("completion-stale");
            return;
        }
        if !matches!(&binding.payload, Payload::Selection { provider, selector, source: Some(source), .. }
            if *provider == reselect.provider && *selector == reselect.selector && source.session_id == context.session)
            || bindings::validate(&binding, &context.root, &context.session).is_err()
        {
            self.completion_error("completion-source-changed");
            return;
        }
        if !self.completion_make_room(&binding) {
            return;
        }
        match &reselect.target.key {
            ReferenceKey::Mark(id) => {
                if !self.completion_replace_binding(&reselect.target.draft, id, binding) {
                    return;
                }
            }
            ReferenceKey::Original { provider, selector } => {
                if !self.revision.completion_resolve_original(
                    &reselect.target.draft,
                    provider,
                    selector,
                    binding,
                ) {
                    self.completion_error("completion-stale");
                    return;
                }
            }
        }
        self.checkpoint_changed(crate::state::Impact::Other);
        self.completion.close();
    }
    fn completion_replace_binding(
        &mut self,
        draft: &DraftKey,
        old: &str,
        binding: Binding,
    ) -> bool {
        let Some(editor) = self.completion_editor(draft) else {
            return false;
        };
        let Some(mark) = editor.marks().iter().find(|mark| mark.id == old) else {
            return false;
        };
        let text = editor.text()[mark.start..mark.end].to_owned();
        let id = uuid::Uuid::new_v4().to_string();
        if !self
            .completion_editor_mut(draft)
            .unwrap()
            .replace_mark(old, &text, &id)
        {
            return false;
        }
        if draft.input.is_some()
            && let Err(key) = self.revision.completion_commit(draft)
        {
            self.completion_editor_mut(draft)
                .unwrap()
                .key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('z'),
                    crossterm::event::KeyModifiers::CONTROL,
                ));
            self.completion_error(key);
            return false;
        }
        self.completion_bindings_mut(draft)
            .unwrap()
            .insert(id, binding);
        true
    }
    pub(super) fn completion_keep_excerpt(&mut self, target: SelectionTarget) {
        if target.draft.input.is_some() && !self.revision.completion_focus(&target.draft) {
            return;
        }
        let ReferenceKey::Mark(id) = &target.key else {
            return;
        };
        let Some(mut binding) = self
            .completion_bindings(&target.draft)
            .and_then(|items| items.get(id))
            .cloned()
        else {
            return;
        };
        let Payload::Selection {
            quote: Some(quote), ..
        } = binding.payload
        else {
            return;
        };
        binding.payload = Payload::Context {
            quote,
            directory: None,
        };
        binding.inline = None;
        if !self.completion_make_room(&binding) {
            return;
        }
        if self.completion_replace_binding(&target.draft, id, binding) {
            self.checkpoint_changed(crate::state::Impact::Other);
            self.completion.close();
        }
    }
    pub(super) fn completion_drop_selection(&mut self, target: SelectionTarget) {
        if target.draft.input.is_some() && !self.revision.completion_focus(&target.draft) {
            return;
        }
        match target.key {
            ReferenceKey::Mark(id) => {
                self.completion_action(Command::Remove {
                    draft: target.draft,
                    id,
                });
            }
            ReferenceKey::Original { provider, selector } => {
                self.revision
                    .completion_drop_original(&target.draft, &provider, &selector);
                self.checkpoint_changed(crate::state::Impact::Other);
            }
        }
    }
}
