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

//! Input-local command discovery and immutable, marked context selections.
mod accept;
mod actions;
pub(crate) mod bindings;
mod candidates;
mod input;
mod io;
mod local;
mod model;
mod query;
mod reselection;
mod results;
mod state;
mod storage;
mod view;
pub use bindings::Checkpoint;
pub use io::{Output, Request, execute};
#[cfg(test)]
pub(crate) use model::Origin;
pub(crate) use model::Payload;
pub use model::{Binding, Category, DraftKey, ReferenceKey, SelectionTarget};
pub(crate) use view::{chips, draw};

use crate::{
    app::{Action, App, ConnectionState},
    editor::{
        Editor,
        completion::{Kind, Token},
    },
    navigation::Route,
};
use model::{Bindings, Context, Popup, Source};
use state::initial;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Open(Kind),
    Resources,
    Category(Category),
    Choose { generation: u64, id: String },
    Preview { generation: u64, id: String },
    Browse { generation: u64, id: String },
    Remove { draft: DraftKey, id: String },
    Added,
    Reselect(SelectionTarget),
    KeepExcerpt(SelectionTarget),
    DropSelection(SelectionTarget),
    Next,
    Previous,
    Retry,
    Close,
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Resources => "completion-plugins",
            Self::Open(Kind::Command) => "completion-commands",
            Self::Open(Kind::Reference) => "completion-context",
            Self::Category(category) => category.label(),
            Self::Choose { .. } => "completion-select",
            Self::Preview { .. } => "completion-preview",
            Self::Browse { .. } => "completion-browse",
            Self::Remove { .. } => "completion-remove",
            Self::Added => "completion-added",
            Self::Reselect(_) => "completion-reselect",
            Self::KeepExcerpt(_) => "completion-keep-excerpt",
            Self::DropSelection(_) => "completion-remove",
            Self::Next => "directory-next",
            Self::Previous => "directory-previous",
            Self::Retry => "list-retry",
            Self::Close => "session-cancel",
        }
    }
}

#[derive(Default)]
pub struct State {
    popup: Option<Popup>,
    bindings: Bindings,
    pending: Option<Request>,
    resolve: Option<io::Job>,
    reselect: Option<model::Reselection>,
    sequence: u64,
    suppressed: Option<(DraftKey, Token)>,
}
impl State {
    pub fn begin_frame(&mut self) {
        if let Some(popup) = &mut self.popup {
            popup.area = None;
        }
    }
    pub fn disconnect(&mut self) {
        if let Some(request) = self.pending.take() {
            request.cancel();
        }
        self.popup = None;
        self.resolve = None;
        self.reselect = None;
        self.suppressed = None;
    }
    pub fn invalidate_geometry(&mut self) {
        if let Some(popup) = &mut self.popup {
            popup.surface.invalidate();
            popup.query_area = None;
            popup.area = None;
        }
    }
    pub fn retained_bytes(&self) -> usize {
        bindings::bytes(&self.bindings)
    }
    pub fn restore(&mut self, saved: Checkpoint) {
        self.bindings = saved
            .drafts
            .into_iter()
            .map(|draft| (draft.key, draft.bindings))
            .collect();
    }
    fn cancel(&mut self) {
        if let Some(request) = &self.pending {
            request.cancel();
        }
    }
    fn close(&mut self) {
        self.cancel();
        self.suppressed = self
            .popup
            .as_ref()
            .map(|popup| (popup.context.draft.clone(), popup.context.token.clone()));
        self.popup = None;
        self.resolve = None;
        self.reselect = None;
    }
}

impl App {
    pub fn completion_cancel(&mut self) {
        self.completion.close();
    }
    pub fn completion_occludes(&self, rect: ratatui::layout::Rect) -> bool {
        matches!(
            self.overlay(),
            None | Some(crate::overlay::Overlay::Revision)
        ) && self.completion.popup.as_ref().is_some_and(|popup| {
            self.completion_current(&popup.context, popup.explicit)
                && popup.area.is_some_and(|area| area.intersects(rect))
        })
    }

    pub(crate) fn completion_bindings(
        &self,
        draft: &DraftKey,
    ) -> Option<&std::collections::BTreeMap<String, Binding>> {
        if draft.input.is_some() {
            self.revision.completion_bindings(draft)
        } else {
            self.completion.bindings.get(draft)
        }
    }
    pub(crate) fn completion_bindings_mut(
        &mut self,
        draft: &DraftKey,
    ) -> Option<&mut std::collections::BTreeMap<String, Binding>> {
        if draft.input.is_some() {
            self.revision.completion_bindings_mut(draft)
        } else {
            Some(self.completion.bindings.entry(draft.clone()).or_default())
        }
    }
    pub fn completion_retained_bytes(&self) -> usize {
        self.completion
            .retained_bytes()
            .saturating_add(self.revision.completion_bytes())
    }

    pub(crate) fn completion_draft(&self) -> Option<DraftKey> {
        if self.revision.visible {
            return self.revision.completion_target();
        }
        let Route::Session(session) = self.navigation.current() else {
            return None;
        };
        self.drafts.contains_key(&session).then_some(DraftKey {
            session,
            input: None,
            display: false,
        })
    }
    pub(crate) fn completion_editor(&self, draft: &DraftKey) -> Option<&Editor> {
        if draft.input.is_some() {
            self.revision.completion_editor(draft)
        } else {
            self.drafts.get(&draft.session)
        }
    }
    pub(crate) fn completion_editor_mut(&mut self, draft: &DraftKey) -> Option<&mut Editor> {
        if draft.input.is_some() {
            self.revision.completion_editor_mut(draft)
        } else {
            self.drafts.get_mut(&draft.session)
        }
    }
    fn completion_context(&self, explicit: Option<Kind>) -> Option<Context> {
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        let draft = self.completion_draft()?;
        let editor = self.completion_editor(&draft)?;
        let token = if let Some(kind) = explicit {
            editor.insertion_token(kind)
        } else {
            editor.completion()?
        };
        if !explicit.is_some()
            && editor
                .marks()
                .iter()
                .any(|mark| mark.start < token.range.end && token.range.start < mark.end)
        {
            return None;
        }
        let session = if draft.input.is_some() {
            self.revision.selections_source()?.to_owned()
        } else {
            draft.session.clone()
        };
        Some(Context {
            root: root_id.clone(),
            epoch: epoch.clone(),
            draft,
            session,
            token,
        })
    }
    fn completion_current(&self, context: &Context, explicit: bool) -> bool {
        self.completion_context(explicit.then_some(context.token.kind))
            .as_ref()
            == Some(context)
    }
    pub fn completion_open(&self) -> bool {
        self.completion.popup.is_some()
    }
}

#[cfg(test)]
mod tests;
