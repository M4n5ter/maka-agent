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
use super::{bindings::SavedDraft, model::Payload};
use maka_protocol::turn::MessageContent;
use maka_runtime::input::{SelectionSources, Selections};
use std::collections::{BTreeMap, HashSet};

impl App {
    pub fn completion_requires_idle(&self, session: &str) -> bool {
        let key = DraftKey {
            session: session.into(),
            input: None,
            display: false,
        };
        self.drafts.get(session).is_some_and(|editor| {
            editor.marks().iter().any(|mark| {
                self.completion
                    .bindings
                    .get(&key)
                    .and_then(|items| items.get(&mark.id))
                    .is_some_and(|binding| {
                        matches!(&binding.payload, Payload::Selection { source: None, .. })
                    })
            })
        })
    }

    pub fn completion_content(
        &self,
        draft: &DraftKey,
        editor: &Editor,
        content: &mut MessageContent,
        selections: &mut Selections,
        sources: &mut SelectionSources,
    ) -> Result<(), &'static str> {
        bindings::merge(
            &self.completion.bindings,
            draft,
            editor,
            content,
            selections,
            sources,
        )
    }
    pub fn completion_checkpoint(&self) -> Option<Checkpoint> {
        let root = match &self.connection {
            ConnectionState::Connected { root_id, .. } => root_id.clone(),
            _ => self.checkpoint_root().to_owned(),
        };
        let drafts = self
            .completion
            .bindings
            .iter()
            .filter_map(|(key, bindings)| {
                let ids: HashSet<_> = self
                    .completion_editor(key)?
                    .marks()
                    .iter()
                    .map(|mark| mark.id.as_str())
                    .collect();
                let bindings: BTreeMap<_, _> = bindings
                    .iter()
                    .filter(|(id, _)| ids.contains(id.as_str()))
                    .map(|(id, binding)| (id.clone(), binding.clone()))
                    .collect();
                (!bindings.is_empty()).then_some(SavedDraft {
                    key: key.clone(),
                    bindings,
                })
            })
            .collect::<Vec<_>>();
        (!drafts.is_empty()).then_some(Checkpoint { root, drafts })
    }
    pub fn completion_gc(&mut self) {
        self.revision.completion_gc();
        let keep: BTreeMap<_, HashSet<String>> = self
            .completion
            .bindings
            .keys()
            .filter_map(|key| {
                Some((
                    key.clone(),
                    self.completion_editor(key)?
                        .retained_mark_ids()
                        .map(str::to_owned)
                        .collect(),
                ))
            })
            .collect();
        self.completion.bindings.retain(|key, bindings| {
            bindings.retain(|id, _| keep.get(key).is_some_and(|keep| keep.contains(id)));
            !bindings.is_empty()
        });
    }
}
