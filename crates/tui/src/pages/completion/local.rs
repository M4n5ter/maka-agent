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

use super::model::{Candidate, Cursor, Pick};
use super::*;

impl App {
    pub(super) fn completion_source(&mut self, source: Source) {
        self.completion.cancel();
        let Some(popup) = &mut self.completion.popup else {
            return;
        };
        popup.source = source;
        popup.cursor = None;
        popup.previous.clear();
        popup.preview = None;
        self.completion_query_changed();
    }
    pub(super) fn completion_query_changed(&mut self) {
        self.completion.cancel();
        self.completion.sequence = self.completion.sequence.wrapping_add(1);
        if let Some(popup) = &mut self.completion.popup {
            popup.generation = self.completion.sequence;
            popup.controls = false;
            popup.candidates.clear();
            popup.selected = None;
            popup.scan = None;
            popup.next = None;
            popup.error = None;
            popup.preview = None;
            popup.requested = true;
            popup.surface.invalidate();
            popup.area = None;
        }
        self.completion.resolve = None;
        self.completion_local();
    }
    pub(super) fn completion_local(&mut self) {
        let Some(popup) = &self.completion.popup else {
            return;
        };
        let result = match popup.source {
            Source::Commands => {
                if matches!(popup.cursor, Some(Cursor::Skills { .. })) {
                    Some((vec![], None))
                } else {
                    let offset = match popup.cursor {
                        Some(Cursor::Local(offset)) => offset,
                        _ => 0,
                    };
                    Some(super::candidates::commands(
                        self,
                        &popup.context,
                        popup.query.text(),
                        offset,
                    ))
                }
            }
            Source::Added => {
                let rows = self
                    .completion_editor(&popup.context.draft)
                    .into_iter()
                    .flat_map(Editor::marks)
                    .filter_map(|mark| {
                        let binding = self
                            .completion_bindings(&popup.context.draft)?
                            .get(&mark.id)?;
                        Some(Candidate {
                            id: format!("bound:{}", mark.id),
                            title: binding.label.clone(),
                            detail: String::new(),
                            source: view::origin(self, &binding.origin),
                            enabled: true,
                            pick: Pick::Bound(mark.id.clone()),
                        })
                    })
                    .collect();
                Some((rows, None))
            }
            _ => None,
        };
        if let Some((rows, next)) = result {
            let skills = matches!(
                self.completion.popup.as_ref().map(|popup| &popup.source),
                Some(Source::Commands)
            ) && next.is_none();
            self.completion_rows(rows, next);
            if skills {
                self.completion.popup.as_mut().unwrap().requested = true;
            }
        }
    }
    pub(super) fn completion_rows(&mut self, rows: Vec<Candidate>, next: Option<Cursor>) {
        let Some(popup) = &mut self.completion.popup else {
            return;
        };
        popup.selected = popup
            .selected
            .take()
            .filter(|id| rows.iter().any(|row| row.id == *id))
            .or_else(|| rows.first().map(|row| row.id.clone()));
        popup.candidates = rows;
        popup.next = next;
        popup.requested = false;
        popup.error = None;
        popup.surface.invalidate();
        popup.area = None;
    }
}
