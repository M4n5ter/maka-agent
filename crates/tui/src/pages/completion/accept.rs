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
use super::{
    io::Job,
    model::{Candidate, Origin, Payload, Pick},
};
use maka_runtime::input::InlineReferenceKind;

impl App {
    pub(super) fn completion_captured(&mut self, request: Request, binding: Binding) {
        if self.completion.reselect.is_some() {
            let source = view::origin(self, &binding.origin);
            let popup = self.completion.popup.as_mut().unwrap();
            let id = "resolved-resource".to_owned();
            popup.controls = false;
            popup.preview = Some(binding.clone());
            popup.selected = Some(id.clone());
            popup.candidates = vec![Candidate {
                id,
                title: binding.label.clone(),
                detail: String::new(),
                source,
                enabled: true,
                pick: Pick::Captured(binding),
            }];
            popup.area = None;
            return;
        }
        if matches!(
            request.job,
            Job::Capture { accept: true, .. } | Job::Resolve { accept: true, .. }
        ) {
            self.completion_bind(request.context, binding);
        } else if let Some(popup) = &mut self.completion.popup {
            let candidate = popup.candidates.iter_mut().find(|candidate| {
                match (&candidate.pick, &request.job) {
                    (Pick::Workspace(current), Job::Capture { input, .. }) => current == input,
                    (
                        Pick::Resource {
                            provider: current,
                            id: current_id,
                        },
                        Job::Resolve { provider, id, .. },
                    ) => {
                        current_id == id
                            && current.provider == provider.provider
                            && current.package_id == provider.package_id
                            && current.method == provider.method
                            && current.target == provider.target
                    }
                    _ => false,
                }
            });
            if let Some(candidate) = candidate {
                candidate.pick = Pick::Captured(binding.clone());
                if popup.selected.as_ref() == Some(&candidate.id) {
                    popup.controls = false;
                    popup.preview = Some(binding);
                    popup.area = None;
                }
            }
        }
    }

    pub(super) fn completion_pick(&mut self, id: &str, accept: bool) -> Option<Action> {
        let popup = self.completion.popup.as_ref()?;
        let candidate = popup
            .candidates
            .iter()
            .find(|candidate| candidate.id == id)?
            .clone();
        let context = popup.context.clone();
        if !self.completion_current(&context, popup.explicit) {
            self.completion.close();
            return None;
        }
        match candidate.pick {
            Pick::Native(action) => {
                if !self.enabled(&action) {
                    return None;
                }
                // Only the reviewed command token is consumed. Arguments and
                // all surrounding draft text remain available to normal forms.
                if !self.completion_consume_command(&context, popup.explicit) {
                    return None;
                }
                self.completion.close();
                return self.apply(action);
            }
            Pick::Command(command) => {
                let action = super::candidates::command_action(self, &context, &command)?;
                if !self.completion_consume_command(&context, popup.explicit) {
                    return None;
                }
                self.completion.close();
                return self.apply(action);
            }
            Pick::Skill { id, name } => self.completion_bind(
                context,
                Binding {
                    label: name,
                    origin: Origin::Skill,
                    inline: (!id.is_empty()
                        && id
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
                    .then_some(InlineReferenceKind::Skill),
                    payload: Payload::Selection {
                        provider: crate::pages::skills::PROVIDER.into(),
                        selector: id,
                        source: None,
                        quote: None,
                    },
                },
            ),
            Pick::Workspace(input) => {
                self.completion.cancel();
                self.completion.resolve = Some(Job::Capture { input, accept });
                self.completion.popup.as_mut()?.requested = false;
            }
            Pick::Session { id, name } => {
                self.completion_source(Source::Messages { session: id, name });
            }
            Pick::Provider(provider) => {
                self.completion_source(Source::Plugin(provider));
            }
            Pick::Resource { provider, id } => {
                self.completion.cancel();
                self.completion.resolve = Some(Job::Resolve {
                    provider,
                    id,
                    accept,
                });
                self.completion.popup.as_mut()?.requested = false;
            }
            Pick::Captured(binding) if accept => {
                if self.completion.reselect.is_some() {
                    self.completion_accept_reselection(binding);
                } else {
                    self.completion_bind(context, binding);
                }
            }
            Pick::Captured(binding) => self.completion.popup.as_mut()?.preview = Some(binding),
            Pick::Bound(id) => {
                let binding = self.completion_bindings(&context.draft)?.get(&id).cloned();
                self.completion.popup.as_mut()?.preview = binding;
            }
        }
        None
    }

    fn completion_consume_command(&mut self, context: &Context, explicit: bool) -> bool {
        if explicit {
            return true;
        }
        if self
            .completion_editor_mut(&context.draft)
            .is_none_or(|editor| !editor.complete(&context.token, ""))
        {
            return false;
        }
        if context.draft.input.is_some()
            && let Err(key) = self.revision.completion_commit(&context.draft)
        {
            self.completion_editor_mut(&context.draft).unwrap().key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('z'),
                    crossterm::event::KeyModifiers::CONTROL,
                ),
            );
            self.completion_error(key);
            return false;
        }
        self.checkpoint_changed(crate::state::Impact::Other);
        true
    }
    pub(super) fn completion_bind(&mut self, context: Context, binding: Binding) {
        if bindings::validate(&binding, &context.root, &context.session).is_err() {
            self.completion_error("completion-input-invalid");
            return;
        }
        if !self.completion_make_room(&binding) {
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let shown = match (&binding.inline, &binding.payload) {
            (Some(InlineReferenceKind::Skill), Payload::Selection { selector, .. }) => {
                format!("/skill:{selector}")
            }
            _ => format!(
                "{}{}",
                if matches!(binding.origin, Origin::Skill) {
                    "/"
                } else {
                    "@"
                },
                binding.label
            ),
        };
        if self
            .completion_editor_mut(&context.draft)
            .is_none_or(|editor| !editor.complete_marked(&context.token, &shown, &id))
        {
            self.completion_error("completion-stale");
            return;
        }
        if context.draft.input.is_some()
            && let Err(key) = self.revision.completion_commit(&context.draft)
        {
            self.completion_editor_mut(&context.draft).unwrap().key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('z'),
                    crossterm::event::KeyModifiers::CONTROL,
                ),
            );
            self.completion_error(key);
            return;
        }
        self.completion_bindings_mut(&context.draft)
            .unwrap()
            .insert(id, binding);
        self.checkpoint_changed(crate::state::Impact::Other);
        self.completion.close();
    }
    pub(super) fn completion_make_room(&mut self, binding: &Binding) -> bool {
        if self
            .completion_retained_bytes()
            .saturating_add(bindings::binding_bytes(binding).saturating_add(36))
            > bindings::BUDGET
        {
            for editor in self.drafts.values_mut() {
                editor.clear_history();
            }
            self.revision.completion_trim_history();
            self.completion_gc();
            self.notice = Some(crate::app::Notice::Local("completion-history-trimmed"));
        }
        if self
            .completion_retained_bytes()
            .saturating_add(bindings::binding_bytes(binding).saturating_add(36))
            > bindings::BUDGET
        {
            self.completion_error("completion-binding-budget");
            return false;
        }
        true
    }

    pub(super) fn completion_error(&mut self, key: &'static str) {
        let text = self.i18n.text(key);
        if let Some(popup) = &mut self.completion.popup {
            popup.error = Some(text);
            popup.requested = false;
        } else {
            self.notice = Some(crate::app::Notice::Local(key));
        }
    }
}
