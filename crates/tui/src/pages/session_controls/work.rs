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

use super::{Page, Target};
use maka_client::{Client, ClientError, RequestFailure};
use maka_protocol::{Operation, context::*, message, navigation, session::*};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Mutation {
    Metadata(SessionMetadataUpdateInput),
    Read(SessionReadMarkerSetInput),
    Compact(ContextCompactInput),
    Interrupt(message::InterruptInput),
    Retract(message::RetractInput),
}
impl Mutation {
    pub(super) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Metadata(input) => {
                decode_session_metadata_update_input(&serde_json::json!(input)).map(|_| ())
            }
            Self::Read(input) => {
                decode_session_read_marker_set_input(&serde_json::json!(input)).map(|_| ())
            }
            Self::Compact(input) => {
                decode_context_compact_input(&serde_json::json!(input)).map(|_| ())
            }
            Self::Interrupt(input) => {
                message::decode_input(Operation::TurnInterrupt, &serde_json::json!(input))
                    .map(|_| ())
            }
            Self::Retract(input) => {
                message::decode_input(Operation::QueueRetract, &serde_json::json!(input))
                    .map(|_| ())
            }
        }
        .map_err(|e| e.to_string())
    }
    fn session(&self) -> Option<&str> {
        match self {
            Self::Metadata(i) => Some(&i.session_id),
            Self::Read(i) => Some(&i.session_id),
            Self::Compact(i) => Some(&i.session_id),
            Self::Interrupt(i) => Some(&i.session_id),
            Self::Retract(i) => Some(&i.session_id),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentCheckpoint {
    pub(super) target: Target,
    pub(super) page: Page,
    pub(super) mutation: Mutation,
}
impl CurrentCheckpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        let t = &self.target;
        if t.root != root
            || t.epoch.is_empty()
            || t.epoch.len() > 128
            || t.name.len() > 4096
            || self.mutation.session().is_some_and(|s| s != t.session)
        {
            return Err("Saved session operation changed its destination".into());
        }
        let valid = match (&self.mutation, self.page) {
            (Mutation::Metadata(i), Page::Metadata) => i.expected_revision == t.revision,
            (Mutation::Read(i), Page::MarkRead) => {
                t.read_message.as_deref() == Some(&i.read_through_message_id)
            }
            (Mutation::Compact(_), Page::Compact) => true,
            (Mutation::Interrupt(i), Page::Interrupt) => {
                i.origin_host_epoch == t.epoch
                    && t.run.as_ref() == Some(&(i.turn_id.clone(), i.run_id.clone()))
            }
            (Mutation::Retract(i), Page::RetractQueue) => i.origin_host_epoch == t.epoch,
            _ => false,
        };
        if !valid {
            return Err("Saved session operation changed its purpose".into());
        }
        self.mutation.validate()
    }
}
/// Old business writes remain immutable receipts; only current core operations run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Checkpoint {
    Current(CurrentCheckpoint),
    Legacy(super::legacy::Checkpoint),
}
impl Checkpoint {
    pub fn validate(&self, root: &str) -> Result<(), String> {
        match self {
            Self::Current(saved) => saved.validate(root),
            Self::Legacy(saved) => saved.validate(root),
        }
    }
    pub(super) fn target(&self) -> Target {
        match self {
            Self::Current(saved) => saved.target.clone(),
            Self::Legacy(saved) => saved.target(),
        }
    }
    pub(super) fn page(&self) -> Page {
        self.current().map_or(Page::Metadata, |saved| saved.page)
    }
    pub(super) fn current(&self) -> Option<&CurrentCheckpoint> {
        match self {
            Self::Current(saved) => Some(saved),
            Self::Legacy(_) => None,
        }
    }
}
#[derive(Clone)]
pub(super) enum Work {
    Session,
    Turns { position: u64, through: Option<u64> },
    Landmarks(String),
    CompactStatus(ContextCompactInput),
    Write(Mutation),
}
#[derive(Clone)]
pub struct Request {
    pub(super) target: Target,
    pub(super) work: Work,
    pub(super) sequence: u64,
}
impl Request {
    pub fn needs_checkpoint(&self) -> bool {
        matches!(self.work, Work::Write(_))
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.sequence == other.sequence && self.target == other.target
    }
}
pub enum Output {
    Session(Box<SessionCatalogProjection>),
    SessionChange(Box<SessionUpdateResult>),
    Turns(navigation::TurnsResult),
    Landmarks(navigation::LandmarksResult),
    Compact(Box<ContextCompactResult>),
    CompactStatus(Box<maka_protocol::turn::TurnSnapshot>),
    Queue,
}
pub async fn execute(client: &Client, request: &Request) -> Result<Output, RequestFailure> {
    let session = &request.target.session;
    match &request.work {
        Work::Session => client
            .session(session)
            .await?
            .map(Output::Session)
            .ok_or_else(|| {
                RequestFailure::NotDispatched(ClientError::Protocol(
                    "Session is no longer available".into(),
                ))
            }),
        Work::Turns { position, through } => client
            .session_turns(navigation::TurnsInput {
                session_id: session.clone(),
                through_sequence: *through,
                position: *position,
                max_contributions: 32,
            })
            .await
            .map(Output::Turns),
        Work::Landmarks(turn) => client
            .session_landmarks(navigation::LandmarksInput {
                session_id: session.clone(),
                turn_id: Some(turn.clone()),
                max_landmarks: 64,
            })
            .await
            .map(Output::Landmarks),
        Work::CompactStatus(input) => client
            .query_turn(maka_protocol::turn::TurnQueryInput {
                session_id: input.session_id.clone(),
                turn_id: input.turn_id.clone(),
            })
            .await
            .map(|turn| Output::CompactStatus(Box::new(turn))),
        Work::Write(mutation) => match mutation {
            Mutation::Metadata(input) => client
                .update_session_metadata(input.clone())
                .await
                .map(|v| Output::SessionChange(Box::new(v))),
            Mutation::Read(input) => client
                .mark_session_read(input.clone())
                .await
                .map(|v| Output::Session(Box::new(v))),
            Mutation::Compact(input) => client
                .compact_context(input.clone())
                .await
                .map(|v| Output::Compact(Box::new(v))),
            Mutation::Interrupt(input) => client
                .interrupt_and_retract(input.clone())
                .await
                .map(|_| Output::Queue),
            Mutation::Retract(input) => client
                .retract_message_queue(input.clone())
                .await
                .map(|_| Output::Queue),
        },
    }
}
pub(super) fn unknown(error: &RequestFailure) -> bool {
    matches!(error, RequestFailure::Unknown(_))
        || matches!(error,
        RequestFailure::Rejected(ClientError::Rejected(error)) if matches!(error.code,
            maka_protocol::OperationErrorCode::CommitOutcomeUnknown | maka_protocol::OperationErrorCode::OutcomeUnknown))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_binds_interrupt_epoch_run_and_revisioned_edits() {
        let mut saved = CurrentCheckpoint {
            page: Page::Interrupt,
            target: Target {
                root: "root".into(),
                epoch: "epoch".into(),
                session: "session".into(),
                name: "Named".into(),
                revision: 3,
                read_message: None,
                run: Some(("turn".into(), "run".into())),
            },
            mutation: Mutation::Interrupt(message::InterruptInput {
                origin_host_epoch: "epoch".into(),
                session_id: "session".into(),
                interrupt_id: "interrupt".into(),
                turn_id: "turn".into(),
                run_id: "run".into(),
            }),
        };
        assert!(saved.validate("root").is_ok());
        assert!(saved.validate("other").is_err());
        saved.target.epoch = "restarted".into();
        assert!(saved.validate("root").is_err());
        saved.page = Page::Metadata;
        saved.mutation = Mutation::Metadata(SessionMetadataUpdateInput {
            session_id: "session".into(),
            expected_revision: 4,
            patch: SessionMetadataPatch {
                name: None,
                labels: None,
                is_flagged: Some(true),
            },
        });
        assert!(saved.validate("root").is_err());
    }
}
