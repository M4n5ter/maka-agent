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

use super::{Action, App};
use crate::{
    app::{ConnectionState, Focus, Notice},
    navigation::Route,
    pages::chat::history::anchor::{Anchor, AnchorRead},
};

#[derive(Default)]
pub(super) struct State {
    root: String,
    epoch: String,
    wanted: Option<Anchor>,
    pending: bool,
}
#[derive(Clone)]
pub struct Request {
    root: String,
    epoch: String,
    read: AnchorRead,
}
pub async fn execute(
    client: &maka_client::Client,
    request: &Request,
) -> Result<maka_client::transcript::TranscriptBatch, String> {
    crate::pages::chat::history::anchor::execute_anchor(client, &request.read).await
}
impl App {
    /// Used by normal history controls and public plugin session-message targets.
    pub fn open_session_anchor(
        &mut self,
        session: String,
        turn: Option<String>,
        message: Option<String>,
        sequence: u64,
    ) -> Option<Action> {
        let anchor = Anchor {
            session: session.clone(),
            turn,
            message,
            sequence,
        };
        if !anchor.valid() {
            self.notice = Some(Notice::Local("controls-anchor-unavailable"));
            return None;
        }
        let ConnectionState::Connected { root_id, epoch } = &self.connection else {
            return None;
        };
        self.session_controls.anchor = State {
            root: root_id.clone(),
            epoch: epoch.clone(),
            wanted: Some(anchor),
            pending: false,
        };
        self.apply(Action::Visit(Route::Session(session)))
    }
    pub fn session_anchor_request(&mut self) -> Option<Request> {
        let state = &mut self.session_controls.anchor;
        let anchor = state.wanted.as_ref()?;
        if self.closing || state.pending {
            return None;
        }
        if self.navigation.current() != Route::Session(anchor.session.clone())
            || !matches!(&self.connection,
            ConnectionState::Connected { root_id, epoch } if root_id == &state.root && epoch == &state.epoch)
        {
            state.wanted = None;
            return None;
        }
        if self.chat.session.as_ref() != Some(&anchor.session)
            || self.chat.subscription.is_none()
            || self.chat.snapshot.is_none()
        {
            return None;
        }
        let anchor = state.wanted.take()?;
        match self.chat.begin_anchor(anchor) {
            Ok(read) => {
                state.pending = true;
                Some(Request {
                    root: state.root.clone(),
                    epoch: state.epoch.clone(),
                    read,
                })
            }
            Err(key) => {
                self.notice = Some(Notice::Local(key));
                None
            }
        }
    }
    pub fn session_anchor_completed(
        &mut self,
        request: Request,
        result: Result<maka_client::transcript::TranscriptBatch, String>,
    ) {
        if !matches!(&self.connection, ConnectionState::Connected { root_id, epoch } if root_id == &request.root && epoch == &request.epoch)
        {
            return;
        }
        self.session_controls.anchor.pending = false;
        match self
            .chat
            .complete_anchor(request.read, result, &self.i18n, self.chrome.ascii)
        {
            Ok(true) => {
                self.reveal_chat();
                self.focus = Focus::Transcript;
                self.checkpoint_changed(crate::state::Impact::Other);
            }
            Ok(false) => {}
            Err(error) => self.notice = Some(Notice::Diagnostic(error)),
        }
    }
}
