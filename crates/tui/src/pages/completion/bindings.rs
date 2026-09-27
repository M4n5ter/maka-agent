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

use super::model::{Binding, Bindings, DraftKey, Origin, Payload};
use crate::editor::Editor;
use maka_protocol::turn::{InlineReference, MessageContent};
use maka_runtime::input::{SelectionSources, Selections};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Retained payload bytes, including references reachable only through Undo.
pub const BUDGET: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub root: String,
    pub drafts: Vec<SavedDraft>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedDraft {
    pub key: DraftKey,
    pub bindings: BTreeMap<String, Binding>,
}
impl Checkpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        if self.root != root || self.bytes() > BUDGET {
            return Err("Invalid completion checkpoint".into());
        }
        let mut seen = HashSet::new();
        for draft in &self.drafts {
            if !seen.insert(&draft.key)
                || (draft.key.display || draft.key.input.is_some())
                || !text(&draft.key.session, 512)
                || draft.key.input.as_ref().is_some_and(|id| !text(id, 512))
            {
                return Err("Invalid completion draft identity".into());
            }
            for (id, binding) in &draft.bindings {
                if !text(id, 128) {
                    return Err("Invalid completion binding identity".into());
                }
                validate(binding, root, &draft.key.session)?;
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }
}

pub(super) fn bytes(bindings: &Bindings) -> usize {
    bindings
        .iter()
        .map(|(key, values)| {
            key.session.len()
                + key.input.as_ref().map_or(0, String::len)
                + values
                    .iter()
                    .map(|(id, binding)| id.len() + binding_bytes(binding))
                    .sum::<usize>()
        })
        .sum()
}
pub(crate) fn binding_bytes(binding: &Binding) -> usize {
    serde_json::to_vec(binding)
        .map_or(usize::MAX, |bytes| bytes.len())
        .saturating_add(std::mem::size_of::<Binding>())
}
fn text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
pub(crate) fn validate(binding: &Binding, root: &str, session: &str) -> Result<(), String> {
    if !text(&binding.label, 4096)
        || match &binding.origin {
            Origin::Workspace | Origin::Skill => false,
            Origin::Session { name } => !text(name, 4096),
            Origin::Plugin { title, package } => !text(title, 4096) || !text(package, 512),
        }
    {
        return Err("Invalid completion label".into());
    }
    let mut content = MessageContent {
        text: "reference".into(),
        display_text: None,
        attachments: None,
        directory_references: None,
        quotes: None,
        inline_references: None,
    };
    let mut selections = Selections::new();
    let mut sources = vec![];
    payload(
        binding,
        &mut content,
        &mut selections,
        &mut sources,
        Merge::Admission,
    )?;
    if content
        .directory_references
        .iter()
        .flatten()
        .any(|directory| directory.host_id != root)
    {
        return Err("Completion directory belongs to another Host".into());
    }
    content
        .validate_admission(true)
        .map_err(|error| error.to_string())?;
    maka_runtime::input::validate_selection_sources(&selections, &sources)
        .map_err(str::to_owned)?;
    maka_runtime::input::validate_selection_session(&sources, session).map_err(str::to_owned)
}

#[derive(Clone, Copy)]
pub(crate) enum Merge {
    Draft { inline: bool },
    Admission,
}

pub(super) fn merge(
    bindings: &Bindings,
    key: &DraftKey,
    editor: &Editor,
    content: &mut MessageContent,
    selections: &mut Selections,
    sources: &mut SelectionSources,
) -> Result<(), &'static str> {
    let empty = BTreeMap::new();
    merge_map(
        bindings.get(key).unwrap_or(&empty),
        editor,
        content,
        selections,
        sources,
        Merge::Admission,
    )?;
    maka_runtime::input::validate_selection_session(sources, &key.session)
        .map_err(|_| "completion-source-changed")
}

pub(crate) fn merge_map(
    bindings: &BTreeMap<String, Binding>,
    editor: &Editor,
    content: &mut MessageContent,
    selections: &mut Selections,
    sources: &mut SelectionSources,
    mode: Merge,
) -> Result<(), &'static str> {
    for mark in editor.marks() {
        let binding = bindings
            .get(&mark.id)
            .ok_or("completion-reference-unavailable")?;
        payload(binding, content, selections, sources, mode)
            .map_err(|_| "completion-source-changed")?;
        if let Some(kind) = &binding.inline
            && !matches!(mode, Merge::Draft { inline: false })
        {
            let value = editor
                .text()
                .get(mark.start..mark.end)
                .ok_or("completion-reference-unavailable")?;
            let reference = InlineReference {
                kind: kind.clone(),
                value: value.into(),
                label: binding.label[..binding.label.floor_char_boundary(200)].to_owned(),
                start: editor.text()[..mark.start].encode_utf16().count() as u64,
            };
            content
                .inline_references
                .get_or_insert_with(Vec::new)
                .push(reference);
        }
    }
    if let Some(references) = &mut content.inline_references {
        references.sort_by_key(|reference| reference.start);
    }
    if matches!(mode, Merge::Admission) {
        content
            .validate_admission(!selections.is_empty())
            .map_err(|_| "completion-input-invalid")?;
        maka_runtime::input::validate_selection_sources(selections, sources)
            .map_err(|_| "completion-input-invalid")?;
    }
    Ok(())
}

pub(crate) fn payload(
    binding: &Binding,
    content: &mut MessageContent,
    selections: &mut Selections,
    sources: &mut SelectionSources,
    mode: Merge,
) -> Result<(), String> {
    let quote = match &binding.payload {
        Payload::Context { quote, directory } => {
            if let Some(directory) = directory {
                let directories = content.directory_references.get_or_insert_with(Vec::new);
                if !directories.contains(directory) {
                    directories.push(directory.clone());
                }
            }
            Some(quote)
        }
        Payload::Selection {
            provider,
            selector,
            source,
            quote,
        } => {
            let selectors = selections.entry(provider.clone()).or_default();
            if !selectors.contains(selector) {
                selectors.push(selector.clone());
            }
            if let Some(source) = source {
                if source.provider != *provider {
                    return Err("Resource provider changed".into());
                }
                if let Some(old) = sources.iter().find(|old| old.provider == *provider) {
                    if old != source {
                        if matches!(mode, Merge::Admission) {
                            return Err("Resource registration changed; select it again".into());
                        }
                        sources.push(source.clone());
                    }
                } else {
                    sources.push(source.clone());
                }
            }
            quote.as_ref()
        }
    };
    if let Some(quote) = quote {
        let quotes = content.quotes.get_or_insert_with(Vec::new);
        if !quotes.contains(quote) {
            quotes.push(quote.clone());
        }
    }
    Ok(())
}
