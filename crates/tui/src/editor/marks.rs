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

use super::{Edit, Editor, MAX_TEXT_BYTES, completion::Token};
use std::{collections::HashSet, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

/// A binding's position in this editor. Its payload remains with the input owner.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub id: String,
    pub start: usize,
    pub end: usize,
}

pub(super) fn bytes(marks: &Vec<Mark>) -> usize {
    marks.capacity() * std::mem::size_of::<Mark>()
        + marks.iter().map(|mark| mark.id.capacity()).sum::<usize>()
}

pub(super) fn validate(marks: &[Mark], text: &str) -> Result<(), &'static str> {
    if marks.is_empty() {
        return Ok(());
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err("Input bindings exceed their byte budget");
    }
    let boundaries: Vec<_> = text
        .grapheme_indices(true)
        .map(|(at, _)| at)
        .chain([text.len()])
        .collect();
    let mut ids = HashSet::new();
    let mut end = 0;
    let mut total = 0usize;
    for mark in marks {
        if mark.id.is_empty()
            || mark.id.len() > 128
            || !mark
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || !ids.insert(&mark.id)
            || mark.start < end
            || mark.start >= mark.end
            || boundaries.binary_search(&mark.start).is_err()
            || boundaries.binary_search(&mark.end).is_err()
        {
            return Err("Invalid saved input binding");
        }
        total = total.saturating_add(mark.id.len() + std::mem::size_of::<Mark>());
        end = mark.end;
    }
    if total > MAX_TEXT_BYTES {
        return Err("Input bindings exceed their byte budget");
    }
    Ok(())
}

pub(super) fn rebase(
    marks: &[Mark],
    range: &Range<usize>,
    inserted: usize,
    boundaries: &[usize],
) -> Vec<Mark> {
    marks
        .iter()
        .filter_map(|mark| {
            let mut mark = mark.clone();
            if range.end <= mark.start {
                mark.start = mark.start - range.len() + inserted;
                mark.end = mark.end - range.len() + inserted;
            } else if range.start < mark.end {
                return None;
            }
            // A combining character inserted at an edge may change that grapheme.
            (boundaries.binary_search(&mark.start).is_ok()
                && boundaries.binary_search(&mark.end).is_ok())
            .then_some(mark)
        })
        .collect()
}

impl Editor {
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Includes undo/redo so the input owner can reclaim only truly unreferenced payloads.
    pub fn retained_mark_ids(&self) -> impl Iterator<Item = &str> {
        self.marks
            .iter()
            .chain(
                self.undo
                    .iter()
                    .flat_map(|edit| edit.before_marks.iter().chain(&edit.after_marks)),
            )
            .chain(
                self.redo
                    .iter()
                    .flat_map(|edit| edit.before_marks.iter().chain(&edit.after_marks)),
            )
            .map(|mark| mark.id.as_str())
    }

    pub fn complete_marked(&mut self, token: &Token, text: &str, id: &str) -> bool {
        if !token.matches(self) {
            return false;
        }
        self.marked_replace(token.range.clone(), text, id, " ")
    }

    /// Explicit source pickers can add a binding without requiring typed trigger text.
    #[cfg(test)]
    pub fn insert_marked(&mut self, text: &str, id: &str) -> bool {
        self.marked_replace(self.selection.range(), text, id, " ")
    }

    /// Reselection changes the binding identity in the same undo transaction.
    pub fn replace_mark(&mut self, old_id: &str, text: &str, new_id: &str) -> bool {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == old_id) else {
            return false;
        };
        self.marked_replace(mark.start..mark.end, text, new_id, "")
    }

    fn marked_replace(&mut self, range: Range<usize>, text: &str, id: &str, suffix: &str) -> bool {
        if text.is_empty()
            || text.chars().any(char::is_control)
            || self.marks.iter().any(|mark| mark.id == id)
            || id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || bytes(&self.marks).saturating_add(id.len() + std::mem::size_of::<Mark>())
                > MAX_TEXT_BYTES
            || self.text.len() - range.len() + text.len() + suffix.len() > self.byte_limit
        {
            return false;
        }
        let inserted = format!("{text}{suffix}");
        let before = self.selection;
        let before_marks = self.marks.clone();
        let revision = self.revision;
        self.replace(range.clone(), &inserted);
        self.marks
            .retain(|mark| mark.end <= range.start || mark.start >= range.start + text.len());
        self.marks.push(Mark {
            id: id.into(),
            start: range.start,
            end: range.start + text.len(),
        });
        self.marks.sort_by_key(|mark| mark.start);
        if self.revision == revision {
            // Binding an already identical visible token is still one semantic edit.
            self.revision = self.revision.wrapping_add(1);
            for edit in self.redo.drain(..) {
                self.history_bytes -= edit.bytes();
            }
            let edit = Edit {
                start: range.start,
                removed: inserted.clone(),
                inserted,
                before,
                after: self.selection,
                before_marks,
                after_marks: self.marks.clone(),
            };
            self.history_bytes += edit.bytes();
            self.undo.push_back(edit);
        } else if let Some(edit) = self.undo.back_mut() {
            self.history_bytes -= edit.bytes();
            edit.after_marks = self.marks.clone();
            self.history_bytes += edit.bytes();
        }
        self.trim_history();
        true
    }

    /// Removing a visible reference and undoing it restores the same binding identity.
    pub fn remove_mark(&mut self, id: &str) -> bool {
        let Some(mark) = self.marks.iter().find(|mark| mark.id == id).cloned() else {
            return false;
        };
        self.replace(mark.start..mark.end, "");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn binding_edits_and_unicode_undo_recover_the_original_identity() {
        let mut editor = Editor::default();
        editor.insert("中文 🦀 @文");
        let token = editor.completion().unwrap();
        assert!(editor.complete_marked(&token, "@文档", "resource-1"));
        let marked = editor.text().to_owned();
        let original = editor.marks().to_vec();
        assert_eq!(&editor.text()[original[0].start..original[0].end], "@文档");
        assert!(editor.replace_mark("resource-1", "@文档", "resource-2"));
        assert_eq!(
            editor.text(),
            marked,
            "reselection adds no extra whitespace"
        );
        assert_eq!(editor.marks()[0].id, "resource-2");
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.marks(), original, "undo keeps the original source");
        editor.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        editor.insert("前缀 ");
        assert_eq!(editor.marks()[0].start, original[0].start + "前缀 ".len());
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), marked);
        assert_eq!(editor.marks(), original);
        assert!(editor.remove_mark("resource-1"));
        assert!(editor.marks().is_empty());
        assert!(editor.retained_mark_ids().any(|id| id == "resource-1"));
        editor.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(editor.marks(), original);
        editor.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert!(editor.marks().is_empty());
        editor.clear_history();
        assert!(!editor.retained_mark_ids().any(|id| id == "resource-1"));
    }

    #[test]
    fn edited_labels_unbind_and_saved_marks_cannot_escape_their_text() {
        let mut editor = Editor::default();
        assert!(editor.insert_marked("@🦀文档", "resource-1"));
        let saved = editor.save();
        let restored = Editor::restore(saved).unwrap();
        assert_eq!(restored.marks(), editor.marks());
        assert!(restored.undo.is_empty());
        editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        editor.insert("改");
        assert!(
            editor.marks().is_empty(),
            "editing the displayed label cannot keep hidden authority"
        );
        let mut saved = restored.save();
        saved.marks[0].end = usize::MAX;
        assert!(Editor::restore(saved).is_err());
        let mut saved = restored.save();
        saved.marks.push(saved.marks[0].clone());
        assert!(Editor::restore(saved).is_err());
    }
}
