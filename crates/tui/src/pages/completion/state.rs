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
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

impl App {
    /// Called after editor events. Only explicit editing opens a new popup;
    /// paste changes the draft without executing or discovering commands.
    pub fn completion_refresh(&mut self, event: &Event) {
        if matches!(event, Event::Paste(_)) {
            self.completion.close();
            if let Some(context) = self.completion_context(None) {
                self.completion.suppressed = Some((context.draft, context.token));
            }
            return;
        }
        let active = matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release &&
            (matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete)
            && !key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META)));
        if self
            .completion
            .popup
            .as_ref()
            .is_some_and(|popup| popup.explicit)
        {
            return;
        }
        let Some(context) = self.completion_context(None) else {
            self.completion.close();
            return;
        };
        if self
            .completion
            .popup
            .as_ref()
            .is_some_and(|popup| popup.context == context)
        {
            return;
        }
        if !active && self.completion.popup.is_none() {
            return;
        }
        if active {
            self.completion.suppressed = None;
        }
        if self.completion.suppressed.as_ref()
            == Some(&(context.draft.clone(), context.token.clone()))
        {
            return;
        }
        let mut source = self
            .completion
            .popup
            .as_ref()
            .filter(|popup| {
                popup.context.draft == context.draft
                    && popup.context.token.kind == context.token.kind
            })
            .map(|popup| popup.source.clone())
            .unwrap_or_else(|| initial(context.token.kind));
        if active && matches!(source, Source::Workspace { .. }) {
            source = Source::Workspace {
                directory: String::new(),
            };
        }
        self.completion_start(context, source, false);
    }

    pub(super) fn completion_start(&mut self, context: Context, source: Source, explicit: bool) {
        if explicit && context.draft.input.is_none() {
            self.focus = crate::app::Focus::Composer;
        }
        self.completion.cancel();
        self.completion.resolve = None;
        self.completion.reselect = None;
        self.completion.sequence = self.completion.sequence.wrapping_add(1);
        let mut query = Editor::bounded(1024, "completion-query-long");
        if !explicit {
            query.insert(&context.token.query);
        }
        self.completion.popup = Some(Popup {
            context,
            generation: self.completion.sequence,
            source,
            explicit,
            controls: false,
            query,
            candidates: vec![],
            selected: None,
            next: None,
            previous: vec![],
            cursor: None,
            scan: None,
            error: None,
            preview: None,
            surface: Default::default(),
            query_area: None,
            area: None,
            requested: true,
        });
        self.completion_local();
    }

    pub fn completion_catalog_changed(&mut self) {
        if self.completion.popup.as_ref().is_some_and(|popup| {
            matches!(
                popup.source,
                Source::Commands | Source::Providers | Source::Plugin(_)
            )
        }) {
            self.completion_query_changed();
        }
    }
}

pub(super) fn initial(kind: Kind) -> Source {
    match kind {
        Kind::Command => Source::Commands,
        Kind::Reference => Source::Workspace {
            directory: String::new(),
        },
    }
}
