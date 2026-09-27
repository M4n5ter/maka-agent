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

impl State {
    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }
    pub fn sync_terminal(
        &mut self,
        client: &maka_client::Client,
        subscription: Option<&str>,
        runner: &mut terminal::Runner,
    ) {
        let target = self.terminal_target(subscription).filter(|target| {
            target.root == client.identity.root_id && target.epoch == client.identity.host_epoch
        });
        self.terminal.bind(target.clone());
        if target.is_some() {
            let output = self
                .selected
                .as_ref()
                .and_then(|reference| self.items.get(reference))
                .and_then(|row| row.result.output.as_ref());
            self.terminal.canonical(output);
        }
        runner.reconcile(client, target);
        self.terminal.flush(runner);
    }
    pub fn terminal_event(&mut self, event: terminal::Event) -> bool {
        let refresh = matches!(
            event.update,
            terminal::Update::Acquired(_) | terminal::Update::Written
        );
        let current = self.terminal.event(event);
        if current && refresh {
            self.invalidate_output();
        }
        current
    }
    pub fn take_copy(&mut self) -> Option<String> {
        self.terminal.copy.take()
    }
    pub fn invalidate_geometry(&mut self) {
        self.command.invalidate_geometry();
    }
    pub fn checkpoint(&self) -> Option<Checkpoint> {
        self.saved.clone()
    }
    pub fn restore(&mut self, saved: Checkpoint) {
        self.saved = Some(saved);
        self.mutation = Mutation::Unknown;
    }
    pub fn disconnect(&mut self) {
        self.pending = None;
        self.busy = false;
        self.visible = false;
        self.terminal.clear();
        self.command = Editor::bounded(32 * 1024, "resources-command-limit");
        self.items.clear();
        self.target = None;
        self.generation += 1;
        self.mutation = if self.saved.is_some() {
            Mutation::Unknown
        } else {
            Mutation::Idle
        };
    }
    pub fn request(&mut self) -> Option<Request> {
        if !self.visible || self.busy {
            return None;
        }
        let work = self.pending.take()?;
        let mut target = self.target.clone()?;
        if matches!(work, Work::Check(_)) {
            target.session = self.saved.as_ref()?.session.clone();
        }
        self.serial += 1;
        let request = Request {
            target,
            generation: self.generation,
            serial: self.serial,
            work,
        };
        self.busy = true;
        if request.needs_checkpoint() {
            self.saved = request.saved();
            self.mutation = Mutation::Saving;
        }
        Some(request)
    }
    pub fn after_checkpoint(&mut self, request: &Request, result: &Result<(), String>) -> bool {
        if self.mutation != Mutation::Saving || self.saved != request.saved() {
            return false;
        }
        if result.is_ok()
            && self.visible
            && self.target.as_ref() == Some(&request.target)
            && self.generation == request.generation
            && self.serial == request.serial
        {
            self.mutation = Mutation::Pending;
            true
        } else {
            self.mutation = Mutation::Unknown;
            self.busy = false;
            self.error = Some("resources-not-dispatched");
            false
        }
    }
    pub(super) fn refresh(&mut self) {
        if let Some(target) = &self.target {
            self.pending = Some(Work::Query(ResourceQueryInput::ListStart {
                session_id: target.session.clone(),
            }));
        }
    }
    pub(super) fn refresh_selected(&mut self) {
        if let (Some(target), Some(reference)) = (&self.target, &self.selected) {
            self.pending = Some(Work::Query(ResourceQueryInput::Get {
                session_id: target.session.clone(),
                resource_ref: reference.clone(),
            }));
        } else {
            self.refresh();
        }
    }
    pub(super) fn selected(&self) -> Option<&ResourceUpdate> {
        self.items.get(self.selected.as_ref()?)
    }
    pub(super) fn running(row: &ResourceUpdate) -> bool {
        matches!(
            row.result.status,
            ShellStatus::Starting | ShellStatus::Running
        )
    }
    pub fn terminal_target(&self, subscription: Option<&str>) -> Option<terminal::Target> {
        let target = self.target.as_ref()?;
        let row = self.selected()?;
        let subscription = subscription?;
        let stopping=matches!(self.pending,Some(Work::Stop(_))) || self.saved.as_ref().is_some_and(|saved| matches!(&saved.operation,Pending::Stop { resource_ref } if resource_ref==&row.result.resource_ref));
        (self.visible
            && !stopping
            && !self.command_form
            && matches!(row.ownership, Ownership::Local)
            && Self::running(row)
            && row.result.mode == ShellMode::Pty)
            .then(|| terminal::Target {
                root: target.root.clone(),
                epoch: target.epoch.clone(),
                session: target.session.clone(),
                subscription: subscription.into(),
                resource_ref: row.result.resource_ref.clone(),
                generation: self.generation,
            })
    }
    fn invalidate_output(&mut self) {
        if self.busy {
            self.stale = true;
        } else if self.pending.is_none() {
            self.refresh_selected();
        }
    }
    pub fn observe(&mut self, frame: &ResourceObservationFrame) {
        let Some(target) = &self.target else {
            return;
        };
        match frame {
            ResourceObservationFrame::DomainChanged {
                host_epoch,
                session_id,
                ..
            } if host_epoch == &target.epoch && session_id == &target.session => {
                self.invalidate_output();
            }
            _ => {}
        }
    }
}
