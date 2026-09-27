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

//! A view of the Host's canonical screen. Raw observation tails are never replayed.
mod input;
mod runner;
use maka_protocol::resource::{ControllerAcquireResult, PtyControl};
use maka_runtime::{
    shell_result::ShellOutput,
    terminal::{
        TerminalCursor, TerminalSize,
        input::{InputAction, encoded_actions_byte_len},
    },
};
pub use runner::Runner;
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub root: String,
    pub epoch: String,
    pub session: String,
    pub subscription: String,
    pub resource_ref: String,
    pub generation: u64,
}
#[derive(Clone)]
pub enum Control {
    Write(PtyControl),
}
pub enum Update {
    Acquired(ControllerAcquireResult),
    Written,
    Failed(&'static str),
    Closed(bool),
}
pub struct Event {
    pub target: Target,
    pub token: Uuid,
    pub update: Update,
}
pub struct Display {
    pub screen: String,
    pub scrollback: String,
    pub size: TerminalSize,
    pub cursor: TerminalCursor,
}
#[derive(Default)]
pub struct View {
    target: Option<Target>,
    token: Option<Uuid>,
    leased: bool,
    size: Option<TerminalSize>,
    pub screen: Option<Display>,
    pub copy: Option<String>,
    pub capture: bool,
    pub scroll: usize,
    pub error: Option<&'static str>,
    syncing: bool,
    queue: VecDeque<(Control, usize)>,
    queued_bytes: usize,
    resize: Option<TerminalSize>,
}
impl View {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn ready(&self) -> bool {
        self.leased
            && self.screen.is_some()
            && self.error.is_none_or(|key| key == "resources-input-full")
    }
    pub fn bind(&mut self, target: Option<Target>) {
        if self.target != target {
            self.clear();
            self.target = target;
            self.syncing = self.target.is_some();
        }
    }
    pub fn canonical(&mut self, output: Option<&ShellOutput>) {
        if matches!(output, Some(ShellOutput::Pty { redacted: true, .. })) {
            self.error = Some("resources-redacted");
            self.capture = false;
        } else if self.error == Some("resources-redacted") {
            self.error = None;
        }
        self.screen = match output {
            Some(ShellOutput::Pty {
                screen,
                scrollback,
                cols,
                rows,
                cursor,
                redacted: false,
                ..
            }) => TerminalSize::new(*cols, *rows).ok().map(|size| Display {
                screen: screen.clone(),
                scrollback: scrollback.clone(),
                size,
                cursor: cursor.clone(),
            }),
            _ => None,
        };
        self.scroll = self.scroll.min(
            self.screen
                .as_ref()
                .map_or(0, |screen| screen.scrollback.lines().count()),
        );
    }
    pub fn event(&mut self, event: Event) -> bool {
        if self.target.as_ref() != Some(&event.target)
            || self.token.is_some_and(|token| token != event.token)
        {
            return false;
        }
        self.token = Some(event.token);
        match event.update {
            Update::Acquired(cut) => {
                // Acquire is a lease/sequence fence. Its bounded raw tail is not a screen.
                self.size = Some(cut.pty.size);
                self.leased = true;
                self.syncing = false;
                self.error = None;
            }
            Update::Written => {}
            Update::Failed(key) => self.fail(key),
            Update::Closed(confirmed) => {
                self.capture = false;
                self.leased = false;
                if !confirmed {
                    self.error = Some("resources-cleanup-unknown");
                }
            }
        }
        true
    }
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let Ok(size) = TerminalSize::new(cols.clamp(2, 240), rows.clamp(1, 100)) else {
            return;
        };
        self.resize = (self.size != Some(size)).then_some(size);
    }
    pub fn flush(&mut self, runner: &Runner) {
        let Some(token) = self.token else {
            return;
        };
        // A held lease can capture and queue input during resize. Only dispatch
        // waits for the ordered resize receipt, so a post-draw flush cannot revoke
        // the capture button that the user just saw or discard their next key.
        if !self.ready() || self.syncing {
            return;
        }
        if let Some(size) = self.resize.take() {
            if runner
                .control(
                    token,
                    Control::Write(PtyControl::Resize {
                        cols: size.cols(),
                        rows: size.rows(),
                    }),
                )
                .is_err()
            {
                self.resize = Some(size);
            } else {
                self.syncing = true;
            }
            return;
        }
        if let Some((control, bytes)) = self.queue.pop_front() {
            if runner.control(token, control.clone()).is_err() {
                self.queue.push_front((control, bytes));
            } else {
                self.queued_bytes = self.queued_bytes.saturating_sub(bytes);
                if self.error == Some("resources-input-full") {
                    self.error = None;
                }
            }
        }
    }
    fn write(&mut self, action: InputAction) {
        let actions = vec![action];
        if !encoded_actions_byte_len(&actions).is_ok_and(|bytes| bytes <= 32 * 1024) {
            self.error = Some("resources-input-full");
            return;
        }
        let control = PtyControl::Actions { actions };
        let bytes = serde_json::to_vec(&control)
            .expect("typed terminal input")
            .len();
        if self.queued_bytes + bytes > 64 * 1024 {
            self.error = Some("resources-input-full");
            return;
        }
        self.queued_bytes += bytes;
        self.queue.push_back((Control::Write(control), bytes));
    }
    fn fail(&mut self, key: &'static str) {
        self.error = Some(key);
        self.capture = false;
        self.leased = false;
        self.queue.clear();
        self.queued_bytes = 0;
    }
}
#[cfg(test)]
mod tests;
