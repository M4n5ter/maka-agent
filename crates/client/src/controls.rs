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

use crate::{Client, ClientError, RequestFailure};
use maka_protocol::{Operation, ProtocolError, Result, context, navigation, session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Non-secret public executor directory. Settings remain the Host's typed session contract.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutorChoices {
    pub revision: u64,
    pub executors: Vec<ExecutorChoice>,
    pub complete: bool,
    pub next_cursor: Option<ExecutorCursor>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutorCursor {
    pub query: String,
    pub generation: uuid::Uuid,
    pub revision: u64,
    pub scope: String,
    pub offset: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorSearchResult {
    Page { page: ExecutorChoices },
    Stale,
}

fn validate_executor_cursor(cursor: &ExecutorCursor) -> Result<()> {
    if cursor.query.len() > 512
        || cursor.query.chars().any(char::is_control)
        || cursor.scope.len() > 264
        || cursor
            .scope
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || !(cursor.scope == "profile"
            || cursor.scope == "desktop-ui"
            || cursor
                .scope
                .strip_prefix("session:")
                .is_some_and(|session| !session.is_empty()))
        || cursor.revision > (1 << 53) - 1
        || cursor.offset > (1 << 53) - 1
    {
        return Err(ProtocolError::invalid("Invalid executor cursor"));
    }
    Ok(())
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutorChoice {
    pub id: String,
    pub display_name: String,
    pub capabilities: ExecutorCapabilities,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutorCapabilities {
    pub thinking: bool,
    pub tool_activity: bool,
    pub attachments: bool,
    pub history_copy: bool,
}

pub(crate) fn supports(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::ContextCompact | Operation::ExecutorCatalogQuery
    ) || navigation::supports(operation)
}
pub(crate) fn decode_input(operation: Operation, value: &Value) -> Result<Value> {
    if navigation::supports(operation) {
        return navigation::decode_input(operation, value);
    }
    if operation == Operation::ContextCompact {
        context::decode_context_compact_input(value)?;
    } else {
        let object = maka_protocol::codec::record(value, "Executor query")?;
        maka_protocol::codec::exact(object, &["scope", "query", "cursor"])?;
        if !value["query"]
            .as_str()
            .is_some_and(|query| query.len() <= 512 && !query.chars().any(char::is_control))
        {
            return Err(ProtocolError::invalid("Invalid executor search query"));
        }
        let scope = maka_protocol::codec::string(&value["scope"], "executor scope", 264)?;
        let session = scope.strip_prefix("session:").unwrap_or("");
        if session.is_empty() || session.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ProtocolError::invalid("Expected a session executor scope"));
        }
        if !value["cursor"].is_null() {
            let cursor: ExecutorCursor = serde_json::from_value(value["cursor"].clone())
                .map_err(|error| ProtocolError::invalid(error.to_string()))?;
            validate_executor_cursor(&cursor)?;
        }
    }
    Ok(value.clone())
}
pub(crate) fn decode_output(operation: Operation, value: &Value) -> Result<Value> {
    if navigation::supports(operation) {
        return navigation::decode_output(operation, value);
    }
    if operation == Operation::ContextCompact {
        context::decode_context_compact_result(value)?;
    } else {
        let result: ExecutorSearchResult = serde_json::from_value(value.clone())
            .map_err(|e| ProtocolError::invalid(e.to_string()))?;
        let ExecutorSearchResult::Page { page: choices } = result else {
            return Ok(value.clone());
        };
        maka_protocol::codec::count(&json!(choices.revision), "executor revision")?;
        if choices.executors.len() > 50
            || choices.complete != choices.next_cursor.is_none()
            || !choices.complete && choices.executors.is_empty()
            || serde_json::to_vec(value)
                .map_err(|error| ProtocolError::invalid(error.to_string()))?
                .len()
                > 48 * 1024
        {
            return Err(ProtocolError::invalid("Invalid executor search page"));
        }
        if let Some(cursor) = &choices.next_cursor {
            validate_executor_cursor(cursor)?;
            if cursor.revision != choices.revision {
                return Err(ProtocolError::invalid(
                    "Invalid executor continuation revision",
                ));
            }
        }
        let mut ids = std::collections::HashSet::new();
        for choice in choices.executors {
            if choice.id.is_empty()
                || choice.id.len() > 128
                || !ids.insert(choice.id)
                || choice.display_name.len() > 4096
            {
                return Err(ProtocolError::invalid(
                    "Invalid executor directory identity",
                ));
            }
        }
    }
    Ok(value.clone())
}
pub(crate) fn errors(operation: Operation) -> &'static [maka_protocol::OperationErrorCode] {
    use maka_protocol::OperationErrorCode as Code;
    if operation == Operation::ContextCompact {
        return maka_protocol::turn::START_ERRORS;
    }
    if operation == Operation::ExecutorCatalogQuery {
        return maka_protocol::configuration::QUERY_ERRORS;
    }
    &[
        Code::HostNotReady,
        Code::HostDraining,
        Code::OperationUnavailable,
        Code::InvalidRequest,
        Code::NotFound,
        Code::PersistenceFailed,
        Code::InternalFailure,
    ]
}

impl Client {
    fn invalid_control(&self, error: impl std::fmt::Display) -> RequestFailure {
        self.disconnect();
        RequestFailure::Unknown(ClientError::Protocol(error.to_string()))
    }
    pub async fn compact_context(
        &self,
        input: context::ContextCompactInput,
    ) -> std::result::Result<context::ContextCompactResult, RequestFailure> {
        let value = self
            .request(Operation::ContextCompact, json!(input))
            .await?;
        let output =
            context::decode_context_compact_result(&value).map_err(|e| self.invalid_control(e))?;
        context::assert_compact_output_for_input(&input, &output)
            .map_err(|e| self.invalid_control(e))?;
        Ok(output)
    }
    pub async fn mark_session_read(
        &self,
        input: session::SessionReadMarkerSetInput,
    ) -> std::result::Result<session::SessionCatalogProjection, RequestFailure> {
        let value = self
            .request(Operation::SessionReadMarkerSet, json!(input))
            .await?;
        let output = session::decode_session_catalog_projection(&value)
            .map_err(|e| self.invalid_control(e))?;
        session::assert_read_marker_output_for_input(&input, &output)
            .map_err(|e| self.invalid_control(e))?;
        Ok(output)
    }
    pub async fn session_executors(
        &self,
        session: &str,
        query: &str,
        cursor: Option<ExecutorCursor>,
    ) -> std::result::Result<ExecutorSearchResult, RequestFailure> {
        let value = self
            .request(
                Operation::ExecutorCatalogQuery,
                json!({"scope":format!("session:{session}"),"query":query,"cursor":cursor}),
            )
            .await?;
        let result: ExecutorSearchResult =
            serde_json::from_value(value).map_err(|e| self.invalid_control(e))?;
        if let ExecutorSearchResult::Page { page } = &result
            && (cursor
                .as_ref()
                .is_some_and(|cursor| page.revision != cursor.revision)
                || page.next_cursor.as_ref().is_some_and(|next| {
                    next.scope != format!("session:{session}")
                        || next.query != query
                        || Some(next.offset)
                            != cursor
                                .as_ref()
                                .map_or(0, |cursor| cursor.offset)
                                .checked_add(page.executors.len() as u64)
                        || cursor.as_ref().is_some_and(|cursor| {
                            next.generation != cursor.generation || next.revision != cursor.revision
                        })
                }))
        {
            return Err(self.invalid_control("Executor page changed its scope, query or cursor"));
        }
        Ok(result)
    }
    pub async fn control_policy(
        &self,
        input: maka_protocol::configuration::policy::RuntimePolicyMutationInput,
    ) -> std::result::Result<
        maka_protocol::configuration::policy::RuntimePolicyMutationResult,
        RequestFailure,
    > {
        use maka_protocol::configuration::policy::RuntimePolicyMutationResult;
        let value = self
            .request(Operation::RuntimePolicyMutate, json!(input))
            .await?;
        let output = maka_protocol::runtime_policy::decode_mutation_result(&value)
            .map_err(|e| self.invalid_control(e))?;
        let valid = match &output {
            RuntimePolicyMutationResult::Committed { revision } => {
                Some(*revision) == input.expected_revision.checked_add(1)
            }
            RuntimePolicyMutationResult::RevisionConflict {
                expected_revision, ..
            } => *expected_revision == input.expected_revision,
        };
        if !valid {
            return Err(self.invalid_control("Policy receipt changed expected revision"));
        }
        Ok(output)
    }
    pub async fn session_turns(
        &self,
        input: navigation::TurnsInput,
    ) -> std::result::Result<navigation::TurnsResult, RequestFailure> {
        let value = self
            .request(Operation::SessionTurnsQuery, json!(input))
            .await?;
        let output: navigation::TurnsResult =
            serde_json::from_value(value).map_err(|e| self.invalid_control(e))?;
        if output.session_id != input.session_id
            || input.through_sequence.is_some() && output.through_sequence != input.through_sequence
            || output.next_position.is_some_and(|p| p <= input.position)
        {
            return Err(self.invalid_control("Turn navigation identity or cursor changed"));
        }
        Ok(output)
    }
    pub async fn session_landmarks(
        &self,
        input: navigation::LandmarksInput,
    ) -> std::result::Result<navigation::LandmarksResult, RequestFailure> {
        let value = self
            .request(Operation::SessionTurnLandmarksQuery, json!(input))
            .await?;
        let output: navigation::LandmarksResult =
            serde_json::from_value(value).map_err(|e| self.invalid_control(e))?;
        if output.session_id != input.session_id
            || input
                .turn_id
                .as_ref()
                .is_some_and(|turn| output.landmarks.iter().any(|item| &item.turn_id != turn))
        {
            return Err(self.invalid_control("Landmark navigation identity changed"));
        }
        Ok(output)
    }
    pub async fn interrupt_and_retract(
        &self,
        input: maka_protocol::message::InterruptInput,
    ) -> std::result::Result<maka_protocol::message::InterruptResult, RequestFailure> {
        let value = self.request(Operation::TurnInterrupt, json!(input)).await?;
        let output = match maka_protocol::message::decode_output(Operation::TurnInterrupt, &value)
            .map_err(|e| self.invalid_control(e))?
        {
            maka_protocol::message::Output::Interrupt(output) => *output,
            _ => return Err(self.invalid_control("Invalid interrupt response")),
        };
        if output.turn.session_id != input.session_id
            || output.turn.turn_id != input.turn_id
            || output.turn.run_id != input.run_id
        {
            return Err(self.invalid_control("Interrupt receipt changed its run"));
        }
        Ok(output)
    }
    pub async fn retract_message_queue(
        &self,
        input: maka_protocol::message::RetractInput,
    ) -> std::result::Result<maka_protocol::message::RetractResult, RequestFailure> {
        let value = self.request(Operation::QueueRetract, json!(input)).await?;
        match maka_protocol::message::decode_output(Operation::QueueRetract, &value)
            .map_err(|e| self.invalid_control(e))?
        {
            maka_protocol::message::Output::Retract(output) => Ok(output),
            _ => Err(self.invalid_control("Invalid retract response")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_executor_search_is_valid_but_foreign_scope_and_control_characters_are_not() {
        let input = json!({"scope":"session:s","query":"","cursor":null});
        assert!(decode_input(Operation::ExecutorCatalogQuery, &input).is_ok());
        assert!(
            decode_input(
                Operation::ExecutorCatalogQuery,
                &json!({"scope":"session:","query":"","cursor":null})
            )
            .is_err()
        );
        assert!(
            decode_input(
                Operation::ExecutorCatalogQuery,
                &json!({"scope":"profile","query":"","cursor":null})
            )
            .is_err()
        );
        assert!(
            decode_input(
                Operation::ExecutorCatalogQuery,
                &json!({"scope":"session:s","query":"line\nbreak","cursor":null})
            )
            .is_err()
        );
    }
}
