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
use super::{io::Job, model::Pick};
use maka_protocol::session::workspace_context as workspace;

impl App {
    pub fn completion_enabled(&self, command: &Command) -> bool {
        match command {
            Command::Reselect(target)
            | Command::KeepExcerpt(target)
            | Command::DropSelection(target) => {
                self.completion_reselection_enabled(target, command)
            }
            Command::Open(_) | Command::Added | Command::Resources => {
                self.completion_context(Some(Kind::Reference)).is_some()
            }
            Command::Remove { draft, id } => {
                self.completion_draft().as_ref() == Some(draft)
                    && self
                        .completion_editor(draft)
                        .is_some_and(|editor| editor.marks().iter().any(|mark| mark.id == *id))
            }
            Command::Close => self.completion.popup.is_some(),
            Command::Choose { generation, id }
            | Command::Preview { generation, id }
            | Command::Browse { generation, id } => {
                self.completion.popup.as_ref().is_some_and(|popup| {
                    popup.generation == *generation
                        && popup.area.is_some()
                        && self.completion.resolve.is_none()
                        && !self.completion.pending.as_ref().is_some_and(|request| {
                            matches!(request.job, Job::Capture { .. } | Job::Resolve { .. })
                        })
                        && self.completion_current(&popup.context, popup.explicit)
                        && popup.candidates.iter().any(|candidate| {
                            let visible = |path: &str| {
                                popup
                                    .surface
                                    .rect(path)
                                    .is_some_and(|rect| !rect.is_empty())
                            };
                            let title = visible(&format!("completion/candidates/{id}/title/name"));
                            let preview = popup.selected.as_ref() == Some(id)
                                && matches!(&candidate.pick, Pick::Captured(binding) if popup.preview.as_ref() == Some(binding))
                                && visible("completion/label")
                                && visible("completion/controls/select");
                            candidate.id == *id && candidate.enabled && (title || preview)
                        })
                })
            }
            Command::Next => self
                .completion
                .popup
                .as_ref()
                .is_some_and(|popup| popup.next.is_some()),
            Command::Previous => self
                .completion
                .popup
                .as_ref()
                .is_some_and(|popup| !popup.previous.is_empty()),
            _ => self.completion.popup.is_some(),
        }
    }

    pub fn completion_action(&mut self, command: Command) -> Option<Action> {
        if !self.completion_enabled(&command) {
            return None;
        }
        match command {
            Command::Open(kind) => {
                let context = self.completion_context(Some(kind))?;
                self.completion_start(context, initial(kind), true);
            }
            Command::Resources => {
                let context = self.completion_context(Some(Kind::Reference))?;
                self.completion_start(context, Source::Providers, true);
            }
            Command::Added => {
                let context = self.completion_context(Some(Kind::Reference))?;
                self.completion_start(context, Source::Added, true);
            }
            Command::Reselect(target) => {
                self.completion_reselect(target);
            }
            Command::KeepExcerpt(target) => {
                self.completion_keep_excerpt(target);
            }
            Command::DropSelection(target) => {
                self.completion_drop_selection(target);
            }
            Command::Close => self.completion.close(),
            Command::Category(category) => {
                self.completion.reselect = None;
                let source = match category {
                    Category::Commands => Source::Commands,
                    Category::Workspace => Source::Workspace {
                        directory: String::new(),
                    },
                    Category::Sessions => Source::Sessions,
                    Category::Plugins => Source::Providers,
                    Category::Added => Source::Added,
                };
                self.completion_source(source);
            }
            Command::Choose { id, .. } => {
                self.completion.popup.as_mut()?.selected = Some(id.clone());
                return self.completion_pick(&id, true);
            }
            Command::Preview { id, .. } => {
                self.completion.popup.as_mut()?.selected = Some(id.clone());
                return self.completion_pick(&id, false);
            }
            Command::Browse { id, .. } => {
                let popup = self.completion.popup.as_ref()?;
                let Pick::Workspace(input) = popup
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id == id)?
                    .pick
                    .clone()
                else {
                    return None;
                };
                if input.kind != workspace::Kind::Directory {
                    return None;
                }
                self.completion.popup.as_mut()?.query =
                    Editor::bounded(1024, "completion-query-long");
                self.completion_source(Source::Workspace {
                    directory: input.path,
                });
            }
            Command::Remove { draft, id } => {
                self.completion_editor_mut(&draft)?.remove_mark(&id);
                if draft.input.is_some()
                    && let Err(key) = self.revision.completion_commit(&draft)
                {
                    self.completion_editor_mut(&draft)?
                        .key(crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Char('z'),
                            crossterm::event::KeyModifiers::CONTROL,
                        ));
                    self.completion_error(key);
                    return None;
                }
                let token = self
                    .completion_editor(&draft)?
                    .insertion_token(Kind::Reference);
                if let Some(popup) = &mut self.completion.popup {
                    popup.context.token = token;
                    popup.explicit = true;
                    popup.preview = None;
                }
                self.checkpoint_changed(crate::state::Impact::Other);
                self.completion_local();
            }
            Command::Next => {
                self.completion.cancel();
                let popup = self.completion.popup.as_mut()?;
                let next = popup.next.take()?;
                popup.previous.push(popup.cursor.take());
                popup.cursor = Some(next);
                self.completion_query_changed();
            }
            Command::Previous => {
                self.completion.cancel();
                let popup = self.completion.popup.as_mut()?;
                popup.cursor = popup.previous.pop()?;
                self.completion_query_changed();
            }
            Command::Retry => {
                let popup = self.completion.popup.as_mut()?;
                popup.cursor = None;
                popup.previous.clear();
                popup.scan = None;
                self.completion_query_changed();
            }
        }
        None
    }
}
