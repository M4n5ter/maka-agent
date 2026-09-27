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

//! Typed native resource operations; identities never resolve through a new selection.
use crate::{Client, ClientError, RequestFailure};
use maka_protocol::{Operation, resource::*, subscription::PtyInterestInput};
use serde_json::Value;

impl Client {
    pub async fn resource_query(
        &self,
        input: ResourceQueryInput,
    ) -> Result<ResourceQueryResult, RequestFailure> {
        let value = self
            .request(
                Operation::RuntimeResourceQuery,
                serde_json::to_value(&input).expect("resource query"),
            )
            .await?;
        let output = decode_query_result(&value).map_err(|error| self.invalid_resource(error))?;
        let valid = match (&input, &output) {
            (
                ResourceQueryInput::ListStart { session_id },
                ResourceQueryResult::Page {
                    session_id: actual, ..
                },
            ) => session_id == actual,
            (
                ResourceQueryInput::ListContinue {
                    session_id,
                    revision,
                    cursor,
                },
                ResourceQueryResult::Page {
                    session_id: actual,
                    revision: got,
                    next_cursor,
                    ..
                },
            ) => session_id == actual && revision == got && next_cursor.as_ref() != Some(cursor),
            (
                ResourceQueryInput::ListContinue { revision, .. },
                ResourceQueryResult::RevisionChanged { expected, actual },
            ) => revision == expected && expected != actual,
            (
                ResourceQueryInput::Get {
                    session_id,
                    resource_ref,
                },
                ResourceQueryResult::Resource {
                    session_id: actual,
                    resource,
                    ..
                },
            ) => {
                session_id == actual
                    && resource
                        .as_ref()
                        .is_none_or(|resource| &resource.result.resource_ref == resource_ref)
            }
            _ => false,
        };
        if !valid {
            return Err(self.invalid_resource("resource query changed identity"));
        }
        Ok(output)
    }
    /// launchId is provenance, not permission to replay an unknown spawn.
    pub async fn resource_start(
        &self,
        input: ResourceStartInput,
    ) -> Result<ResourceMutationResult, RequestFailure> {
        let value = self
            .request(
                Operation::RuntimeResourceStart,
                serde_json::to_value(input).expect("resource start"),
            )
            .await?;
        decode_mutation_result(&value).map_err(|error| self.invalid_resource(error))
    }
    pub async fn resource_stop(&self, input: ResourceStopInput) -> Result<(), RequestFailure> {
        let value = self
            .request(
                Operation::RuntimeResourceStop,
                serde_json::to_value(input).expect("resource stop"),
            )
            .await?;
        if value.as_object().is_none_or(|object| !object.is_empty()) {
            return Err(self.invalid_resource("invalid resource stop receipt"));
        }
        Ok(())
    }
    /// Retain Acquire through its actual reply so late ownership can be released.
    pub async fn resource_controller_acquire(
        &self,
        input: ControllerIdentity,
    ) -> Result<ControllerAcquireResult, RequestFailure> {
        let value = self
            .request_pending(
                Operation::RuntimeResourceControllerAcquire,
                serde_json::to_value(&input).expect("controller identity"),
            )
            .await?
            .settle()
            .await?;
        validate_controller_output(Operation::RuntimeResourceControllerAcquire, &value)
            .map_err(|error| self.invalid_resource(error))?;
        let output: ControllerAcquireResult =
            serde_json::from_value(value).map_err(|error| self.invalid_resource(error))?;
        if output.controller_id != input.controller_id
            || output.pty.session_id != input.session_id
            || output.pty.resource_ref != input.resource_ref
        {
            return Err(self.invalid_resource("controller acquisition changed identity"));
        }
        Ok(output)
    }
    /// Accepted bytes settle before the owner can release this controller.
    pub async fn resource_controller_control(
        &self,
        input: ControllerControlInput,
    ) -> Result<ControllerControlResult, RequestFailure> {
        let value = self
            .request_pending(
                Operation::RuntimeResourceControllerControl,
                serde_json::to_value(&input).expect("controller input"),
            )
            .await?
            .settle()
            .await?;
        validate_controller_output(Operation::RuntimeResourceControllerControl, &value)
            .map_err(|error| self.invalid_resource(error))?;
        let output: ControllerControlResult =
            serde_json::from_value(value).map_err(|error| self.invalid_resource(error))?;
        if output.controller_id != input.controller_id || output.sequence != input.sequence {
            return Err(self.invalid_resource("controller receipt changed identity"));
        }
        Ok(output)
    }
    pub async fn resource_controller_release(
        &self,
        input: ControllerIdentity,
    ) -> Result<ControllerReleaseResult, RequestFailure> {
        let value = self
            .request_pending(
                Operation::RuntimeResourceControllerRelease,
                serde_json::to_value(&input).expect("controller release"),
            )
            .await?
            .settle()
            .await?;
        validate_controller_output(Operation::RuntimeResourceControllerRelease, &value)
            .map_err(|error| self.invalid_resource(error))?;
        let output: ControllerReleaseResult =
            serde_json::from_value(value).map_err(|error| self.invalid_resource(error))?;
        if output.controller_id != input.controller_id {
            return Err(self.invalid_resource("controller release changed identity"));
        }
        Ok(output)
    }
    pub async fn resource_pty_interest(
        &self,
        input: PtyInterestInput,
    ) -> Result<(), RequestFailure> {
        let value = self
            .request(
                Operation::SubscriptionPtyInterestSet,
                serde_json::to_value(&input).expect("PTY interests"),
            )
            .await?;
        let output = maka_protocol::subscription::decode_subscription_close_result(&value)
            .map_err(|error| self.invalid_resource(error))?;
        if output.subscription_id != input.subscription_id {
            return Err(self.invalid_resource("PTY interest changed subscription"));
        }
        Ok(())
    }
    fn invalid_resource(&self, error: impl std::fmt::Display) -> RequestFailure {
        self.disconnect();
        RequestFailure::Unknown(ClientError::Protocol(error.to_string()))
    }
}

/// Shared registry hooks use the same validators as Host admission.
pub(crate) fn decode_input(operation: Operation, value: &Value) -> maka_protocol::Result<Value> {
    match operation {
        Operation::RuntimeResourceQuery => {
            decode_query_input(value)?;
        }
        Operation::RuntimeResourceStart => {
            decode_start_input(value)?;
        }
        Operation::RuntimeResourceStop => {
            decode_stop_input(value)?;
        }
        Operation::RuntimeResourceControllerAcquire
        | Operation::RuntimeResourceControllerRelease => {
            decode_controller_identity(value)?;
        }
        Operation::RuntimeResourceControllerControl => {
            decode_controller_control(value)?;
        }
        _ => {
            return Err(maka_protocol::ProtocolError::invalid(
                "not a resource operation",
            ));
        }
    }
    Ok(value.clone())
}
pub(crate) fn decode_output(operation: Operation, value: &Value) -> maka_protocol::Result<Value> {
    match operation {
        Operation::RuntimeResourceQuery => {
            decode_query_result(value)?;
        }
        Operation::RuntimeResourceStart => {
            decode_mutation_result(value)?;
        }
        Operation::RuntimeResourceStop => {
            if value.as_object().is_none_or(|object| !object.is_empty()) {
                return Err(maka_protocol::ProtocolError::invalid(
                    "invalid resource stop receipt",
                ));
            }
        }
        _ if is_controller(operation) => validate_controller_output(operation, value)?,
        _ => {
            return Err(maka_protocol::ProtocolError::invalid(
                "not a resource operation",
            ));
        }
    }
    Ok(value.clone())
}
pub(crate) fn errors(operation: Operation) -> Option<&'static [maka_protocol::OperationErrorCode]> {
    if operation == Operation::RuntimeResourceQuery {
        Some(QUERY_ERRORS)
    } else if is_controller(operation)
        || matches!(
            operation,
            Operation::RuntimeResourceStart | Operation::RuntimeResourceStop
        )
    {
        Some(MUTATION_ERRORS)
    } else {
        None
    }
}
