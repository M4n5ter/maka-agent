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
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Attempt {
    pub id: Uuid,
    pub phase: Phase,
    pub agent: Option<Agent>,
    pub diagnostic: Option<String>,
    pub stop: CancellationToken,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Installing,
    Cancelling,
    Installed,
    Cancelled,
    Failed,
    Unknown,
}
impl Attempt {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            phase: Phase::Installing,
            agent: None,
            diagnostic: None,
            stop: CancellationToken::new(),
        }
    }
    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Installing | Phase::Cancelling)
    }
}
fn update(state: &watch::Sender<Option<Attempt>>, id: Uuid, change: impl FnOnce(&mut Attempt)) {
    state.send_if_modified(|value| {
        if let Some(attempt) = value.as_mut().filter(|attempt| attempt.id == id) {
            change(attempt);
            true
        } else {
            false
        }
    });
}
pub(super) async fn run(
    setup: &crate::setup::Provider,
    caller: &Caller,
    state: &watch::Sender<Option<Attempt>>,
    attempt: &Attempt,
    stop: &CancellationToken,
    stopping: &CancellationToken,
) -> Result<(), Error> {
    let stream = match setup
        .open(
            json!({"kind":"install_antigravity","operationId":attempt.id}),
            caller.clone(),
        )
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            update(state, attempt.id, |attempt| {
                attempt.phase = Phase::Failed;
                attempt.diagnostic = Some(error.to_string());
            });
            return Err(error);
        }
    };
    consume(stream, state, attempt, stop, stopping).await
}

async fn consume(
    stream: Box<dyn remote::Stream>,
    state: &watch::Sender<Option<Attempt>>,
    attempt: &Attempt,
    stop: &CancellationToken,
    stopping: &CancellationToken,
) -> Result<(), Error> {
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(Error::Cancelled),
        _ = stopping.cancelled() => Err(Error::Cancelled),
        _ = attempt.stop.cancelled() => Err(Error::Cancelled),
        result = async {
            let mut installed = None;
            while let Some(event) = stream.next().await? {
                if event["kind"] == "installed" {
                    let agent: Agent = serde_json::from_value(event["agent"].clone()).map_err(|error| Error::Provider(error.to_string()))?;
                    agent.validate().map_err(|error| Error::Provider(error.to_string()))?;
                    installed = Some(agent);
                }
            }
            installed.ok_or_else(|| Error::Provider("Installation ended without a configuration to adopt".into()))
        } => result,
    };
    if matches!(result, Err(Error::Cancelled)) {
        update(state, attempt.id, |attempt| {
            attempt.phase = Phase::Cancelling
        });
    }
    stream.cancel();
    let cleanup = stream.close().await;
    update(state, attempt.id, |attempt| {
        attempt.agent = None;
        match (&cleanup, &result) {
            (Ok(()), Ok(agent)) => {
                attempt.phase = Phase::Installed;
                attempt.agent = Some(agent.clone());
            }
            (Ok(()), Err(Error::Cancelled)) => attempt.phase = Phase::Cancelled,
            (Err(_), _) | (_, Err(Error::OutcomeUnknown(_) | Error::CleanupUnconfirmed)) => {
                attempt.phase = Phase::Unknown;
                attempt.diagnostic = Some("Installation or cleanup could not be confirmed".into());
            }
            (_, Err(error)) => {
                attempt.phase = Phase::Failed;
                attempt.diagnostic = Some(error.to_string());
            }
        }
    });
    cleanup.and(result.map(|_| ()))
}
pub(super) fn cancel(
    pages: &auth::Pages,
    submission: &Submission,
    caller: &Caller,
) -> Result<Reply, Error> {
    let id = submission
        .action
        .strip_prefix("cancel-install-")
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or(Error::Cancelled)?;
    pages.cancel_install(caller, id)?;
    Ok(Reply::Updated {})
}
pub(super) async fn adopt(
    this: &Agents,
    mut configuration: Configuration,
    submission: &Submission,
    caller: &Caller,
) -> Result<Reply, Error> {
    let attempt = this
        .pages
        .installation(caller)
        .filter(|attempt| {
            submission.action == format!("adopt-install-{}", attempt.id)
                && attempt.phase == Phase::Installed
        })
        .ok_or_else(|| Error::Invalid("This installation result is no longer available".into()))?;
    let agent = attempt.agent.ok_or(Error::Cancelled)?;
    let id = agent.id.clone();
    match configuration
        .agents
        .iter_mut()
        .find(|current| current.id == id)
    {
        Some(current) => *current = agent,
        None => configuration.agents.push(agent),
    }
    this.management.call(json!({"kind":"configure","expectedRevision":configuration.revision,"agents":configuration.agents}), caller.clone()).await?;
    Ok(Reply::Applied {
        route: json!({"agent":id}),
    })
}
mod presentation;
pub(super) use presentation::view;

#[cfg(test)]
mod tests;
