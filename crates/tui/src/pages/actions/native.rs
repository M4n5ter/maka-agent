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

//! Context menus for owner-drawn conversation text and the composer.
use super::*;
use crate::{app::Focus, ui::transcript::selection::CopyMode};
use ratatui::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Cut,
    Copy,
    SelectAll,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorTarget {
    session: String,
    revision: u64,
    selection: std::ops::Range<usize>,
    pub command: Edit,
}
impl EditorTarget {
    pub(crate) fn label(&self) -> &'static str {
        match self.command {
            Edit::Cut => "context-cut",
            Edit::Copy => "context-copy",
            Edit::SelectAll => "context-select-all",
        }
    }
    pub(crate) fn current(&self, app: &App) -> bool {
        app.navigation.current() == Route::Session(self.session.clone())
            && !app.closing
            && app.drafts.get(&self.session).is_some_and(|editor| {
                editor.revision() == self.revision
                    && editor.selection_range() == self.selection
                    && match self.command {
                        Edit::SelectAll => !editor.text().is_empty(),
                        _ => !self.selection.is_empty(),
                    }
            })
    }
    pub(crate) fn text(&self, app: &App) -> Option<String> {
        self.current(app)
            .then(|| app.drafts[&self.session].selected_text().to_owned())
    }
    pub(crate) fn cut_completed(&self, app: &mut App, result: Result<(), &'static str>) -> bool {
        let changed = result.is_ok() && self.command == Edit::Cut && self.current(app);
        if changed {
            self.apply(app);
        }
        app.notice = Some(crate::app::Notice::Clipboard {
            key: result.map_or_else(|key| key, |_| "context-cut-copied"),
            until: std::time::Instant::now() + std::time::Duration::from_secs(3),
        });
        changed
    }
    pub(crate) fn apply(&self, app: &mut App) {
        if !self.current(app) {
            return;
        }
        let editor = app.drafts.get_mut(&self.session).unwrap();
        match self.command {
            Edit::SelectAll => editor.select_all(),
            Edit::Cut => {
                editor.insert("");
            }
            Edit::Copy => {}
        }
        app.focus = Focus::Composer;
        app.checkpoint_changed(crate::state::Impact::Other);
    }
}

pub(crate) fn bind(frame: &Frame<'_>, app: &mut App) {
    let mut regions = Vec::new();
    if let Route::Session(session) = app.navigation.current()
        && app.overlay().is_none()
        && !app.completion_open()
    {
        if !app.chrome.details
            && !app.inspector_replaces_chat()
            && let Some(reader) = app.chat.reader()
        {
            for (key, rect) in reader.message_regions() {
                let Some(target) = CopyTarget::message(app, key.clone()) else {
                    continue;
                };
                let mut commands = vec![
                    (Action::Copy(CopyMode::Selection), "context-copy-selection"),
                    (
                        Action::CopyMessage {
                            target: target.clone(),
                            mode: CopyMode::Message,
                        },
                        "chat-copy-message",
                    ),
                    (
                        Action::CopyMessage {
                            target,
                            mode: CopyMode::Source,
                        },
                        "context-copy-markdown",
                    ),
                ];
                if reader.can_toggle(&key)
                    && let Some(action) = app.chat.effect(
                        crate::ui::transcript::Effect::Disclosure(key.clone()),
                        app.chat.history_scope(),
                    )
                {
                    commands.push((
                        action,
                        if reader.folded(&key) {
                            "context-expand"
                        } else {
                            "context-collapse"
                        },
                    ));
                }
                let id = format!(
                    "message/{session}/{}/{}",
                    app.chat.history_scope(),
                    serde_json::to_string(&key).expect("message key")
                );
                regions.push((id.clone(), rect, context(app, &id, commands)));
            }
        }
        if let Some(rect) = app.chrome.composer.rect("composer/body/content/editor")
            && let Some(editor) = app.drafts.get(&session)
        {
            let target = |command| EditorTarget {
                session: session.clone(),
                revision: editor.revision(),
                selection: editor.selection_range(),
                command,
            };
            let mut commands = vec![
                (Action::EditComposer(target(Edit::Copy)), "context-copy"),
                (
                    Action::Attachment(crate::pages::attachments::Command::Paste),
                    "context-paste",
                ),
                (
                    Action::EditComposer(target(Edit::SelectAll)),
                    "context-select-all",
                ),
            ];
            // clipboard-rs's X11 writer owns a permanent thread and prints to
            // stdout on ownership changes. Do not expose destructive editing
            // until that platform has a terminal-safe confirmed writer.
            #[cfg(any(target_os = "macos", windows))]
            commands.insert(0, (Action::EditComposer(target(Edit::Cut)), "context-cut"));
            commands.extend(composer_add().into_iter().take(3));
            let id = format!(
                "composer/{session}/{}/{:?}",
                editor.revision(),
                editor.selection_range()
            );
            regions.push(("composer".into(), rect, context(app, &id, commands)));
        }
    }
    app.chrome.context.context_regions(frame.area(), regions);
}
