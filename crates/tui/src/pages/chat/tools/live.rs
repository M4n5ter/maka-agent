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

//! A bounded, disposable overlay until the corresponding durable rows arrive.
use super::*;
use maka_protocol::subscription::{SessionToolEvent, ToolResultStatus};
use maka_runtime::display::Text;

pub(in crate::pages::chat) struct Update {
    pub turn: String,
    pub name: String,
    pub title: Option<Text>,
    pub progress: String,
    pub state: State,
    pub revision: u64,
}
#[derive(Default)]
pub(in crate::pages::chat) struct Live {
    pub entries: BTreeMap<String, Update>,
    revision: u64,
}
impl Live {
    pub fn get(&self, turn: &str, id: &str) -> Option<&Update> {
        self.entries.get(id).filter(|update| update.turn == turn)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn finish(&mut self) {
        for update in self.entries.values_mut() {
            if update.state == State::Pending {
                self.revision += 1;
                update.revision = self.revision;
                update.state = State::Missing;
                update.progress.clear();
            }
        }
    }
    pub fn accept(&mut self, event: SessionToolEvent) {
        let (turn, id) = match &event {
            SessionToolEvent::ToolStart {
                turn_id,
                tool_use_id,
                ..
            }
            | SessionToolEvent::ToolProgress {
                turn_id,
                tool_use_id,
                ..
            }
            | SessionToolEvent::ToolResult {
                turn_id,
                tool_use_id,
                ..
            } => (turn_id, tool_use_id),
        };
        if !self.entries.contains_key(id) && self.entries.len() >= 256 {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, update)| update.state != State::Pending)
                .min_by_key(|(_, update)| update.revision)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.entries.remove(&id);
            } else {
                return;
            }
        }
        self.revision += 1;
        let update = self.entries.entry(id.clone()).or_insert_with(|| Update {
            turn: turn.clone(),
            name: String::new(),
            title: None,
            progress: String::new(),
            state: State::Pending,
            revision: 0,
        });
        if update.turn != *turn || update.state != State::Pending {
            return;
        }
        update.revision = self.revision;
        match event {
            SessionToolEvent::ToolStart {
                tool_name, title, ..
            } => {
                update.name = tool_name;
                update.title = title;
            }
            SessionToolEvent::ToolProgress { chunk, .. } => {
                // Keep a bounded trailing line for the card; full evidence stays
                // with its source. Never accumulate a second tool-output log.
                if let Some(line) = chunk.lines().rev().find(|line| !line.trim().is_empty()) {
                    let line = crate::view::safe(line);
                    update.progress = line[..line.floor_char_boundary(1024.min(line.len()))].into();
                }
            }
            SessionToolEvent::ToolResult { status, .. } => {
                update.state = if status == ToolResultStatus::Completed {
                    State::Returned
                } else {
                    State::Attention
                };
                update.progress.clear();
            }
        }
    }
    pub fn reconcile(&mut self, rows: &BTreeMap<u64, Value>) {
        let ids = |kind: &str| {
            rows.values()
                .filter(|row| row["type"] == kind)
                .filter_map(|row| Some((row["turnId"].as_str()?, row["toolUseId"].as_str()?)))
                .collect::<std::collections::HashSet<_>>()
        };
        let results = ids("tool_result");
        let titles = ids("tool_activity");
        self.entries.retain(|id, update| {
            let key = (update.turn.as_str(), id.as_str());
            !results.contains(&key) || (update.title.is_some() && !titles.contains(&key))
        });
    }
}
