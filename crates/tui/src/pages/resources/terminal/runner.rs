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
use maka_client::Client;
use maka_protocol::resource::{ControllerControlInput, ControllerIdentity};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};

pub struct Runner {
    current: Option<Running>,
    retiring: Option<(Target, watch::Receiver<Option<bool>>)>,
    tasks: JoinSet<bool>,
    events: mpsc::UnboundedSender<Event>,
    failed: bool,
}
struct Running {
    target: Target,
    token: Uuid,
    commands: mpsc::Sender<Control>,
    _stop: oneshot::Sender<()>,
    closed: watch::Receiver<Option<bool>>,
}
impl Runner {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (events, receiver) = mpsc::unbounded_channel();
        (
            Self {
                current: None,
                retiring: None,
                tasks: JoinSet::new(),
                events,
                failed: false,
            },
            receiver,
        )
    }
    pub fn reconcile(&mut self, client: &Client, target: Option<Target>) {
        while let Some(result) = self.tasks.try_join_next() {
            self.failed |= !matches!(result, Ok(true));
        }
        if self.current.as_ref().map(|running| &running.target) == target.as_ref() {
            return;
        }
        let previous = self.current.take();
        if let Some(previous) = previous {
            self.retiring = Some((previous.target.clone(), previous.closed.clone()));
            drop(previous);
        }
        let preceding = self
            .retiring
            .as_ref()
            .filter(|(old, _)| {
                target
                    .as_ref()
                    .is_some_and(|target| old.root == target.root && old.epoch == target.epoch)
            })
            .map(|(_, closed)| closed.clone());
        let Some(target) = target else {
            return;
        };
        let token = Uuid::new_v4();
        let (commands, receiver) = mpsc::channel(1);
        let (stop, stopped) = oneshot::channel();
        let (closed, settled) = watch::channel(None);
        self.current = Some(Running {
            target: target.clone(),
            token,
            commands,
            _stop: stop,
            closed: settled,
        });
        let client = client.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            let result = follow(
                client,
                target.clone(),
                token,
                receiver,
                stopped,
                preceding,
                &events,
            )
            .await;
            let _ = events.send(Event {
                target,
                token,
                update: Update::Closed(result),
            });
            let _ = closed.send(Some(result));
            result
        });
    }
    pub fn control(&self, token: Uuid, control: Control) -> Result<(), ()> {
        let running = self
            .current
            .as_ref()
            .filter(|running| running.token == token)
            .ok_or(())?;
        running.commands.try_send(control).map_err(|_| ())
    }
    pub fn stop(&mut self) {
        self.current = None;
        self.retiring = None;
    }
    pub async fn shutdown(&mut self) -> bool {
        self.stop();
        while let Some(result) = self.tasks.join_next().await {
            self.failed |= !matches!(result, Ok(true));
        }
        !self.failed
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.stop();
        self.tasks.detach_all();
    }
}
async fn follow(
    client: Client,
    target: Target,
    token: Uuid,
    mut commands: mpsc::Receiver<Control>,
    mut stopped: oneshot::Receiver<()>,
    preceding: Option<watch::Receiver<Option<bool>>>,
    events: &mpsc::UnboundedSender<Event>,
) -> bool {
    if client.identity.root_id != target.root || client.identity.host_epoch != target.epoch {
        return true;
    }
    if let Some(mut preceding) = preceding {
        loop {
            if let Some(closed) = *preceding.borrow_and_update() {
                if !closed {
                    return false;
                }
                break;
            }
            if preceding.changed().await.is_err() {
                return false;
            }
        }
    }
    if !matches!(stopped.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
        return true;
    }
    let identity = ControllerIdentity {
        session_id: target.session.clone(),
        resource_ref: target.resource_ref.clone(),
        controller_id: token.to_string(),
    };
    // Canonical screen changes arrive through the session's ordered resource
    // observation. Subscribing to raw PTY output duplicates that work and can
    // flood the UI with invisible terminal control strings.
    let acquired = client
        .resource_controller_acquire(identity.clone())
        .await
        .ok();
    let mut sequence = 0;
    let active = if let Some(cut) = acquired {
        sequence = cut.next_sequence;
        match stopped.try_recv() {
            Err(oneshot::error::TryRecvError::Empty) => events
                .send(Event {
                    target: target.clone(),
                    token,
                    update: Update::Acquired(cut),
                })
                .is_ok(),
            _ => false,
        }
    } else {
        let _ = events.send(Event {
            target: target.clone(),
            token,
            update: Update::Failed("resources-terminal-failed"),
        });
        false
    };
    if active {
        loop {
            let command =
                tokio::select! { biased; _=&mut stopped=>None, command=commands.recv()=>command };
            let Some(command) = command else {
                break;
            };
            let update = match command {
                Control::Write(control) => {
                    let resize = control.parts().ok().is_some_and(|(_, size)| size.is_some());
                    match client
                        .resource_controller_control(ControllerControlInput {
                            session_id: identity.session_id.clone(),
                            resource_ref: identity.resource_ref.clone(),
                            controller_id: identity.controller_id.clone(),
                            sequence,
                            control,
                        })
                        .await
                    {
                        Ok(_) => {
                            sequence += 1;
                            if resize {
                                match client.resource_controller_acquire(identity.clone()).await {
                                    Ok(cut) => {
                                        sequence = cut.next_sequence;
                                        Update::Acquired(cut)
                                    }
                                    Err(_) => Update::Failed("resources-terminal-failed"),
                                }
                            } else {
                                Update::Written
                            }
                        }
                        Err(maka_client::RequestFailure::Rejected(
                            maka_client::ClientError::Rejected(error),
                        )) if error.code == maka_protocol::OperationErrorCode::InvalidRequest => {
                            Update::Failed("resources-input-rejected")
                        }
                        Err(_) => Update::Failed("resources-control-unknown"),
                    }
                }
            };
            let failed = matches!(update, Update::Failed(_));
            let _ = events.send(Event {
                target: target.clone(),
                token,
                update,
            });
            if failed {
                break;
            }
        }
    }
    // Even a late/uncertain Acquire uses this known identity and must settle Release.
    client.resource_controller_release(identity).await.is_ok()
}
