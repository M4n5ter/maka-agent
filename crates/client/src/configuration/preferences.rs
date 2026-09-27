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
use maka_protocol::{
    Operation,
    configuration::{
        CredentialState,
        policy::{
            RuntimePolicySnapshot, network_test,
            network_update::{self, CredentialUpdate, Update, UpdateResult},
        },
    },
};

impl Client {
    pub async fn runtime_policy(&self) -> Result<RuntimePolicySnapshot, RequestFailure> {
        let value = self
            .request(Operation::RuntimePolicyQuery, serde_json::json!({}))
            .await?;
        maka_protocol::runtime_policy::decode_query_result(&value)
            .map_err(|_| self.preferences_invalid())
    }

    pub async fn update_network_proxy(
        &self,
        mut input: Update,
    ) -> Result<UpdateResult, RequestFailure> {
        input.normalize().map_err(|_| {
            RequestFailure::NotDispatched(ClientError::Protocol("Invalid proxy settings".into()))
        })?;
        let value = self
            .request(
                Operation::RuntimePolicyNetworkProxyUpdate,
                serde_json::to_value(&input).expect("wire input"),
            )
            .await?;
        let result = maka_protocol::runtime_policy::decode_network_proxy_result(&value)
            .map_err(|_| self.preferences_invalid())?;
        let valid = match &result {
            UpdateResult::Committed {
                revision,
                credential_status,
            } => {
                (*revision == input.expected_policy_revision
                    || Some(*revision) == input.expected_policy_revision.checked_add(1))
                    && credential_status.locator == network_update::locator()
                    && match (
                        &input.credential,
                        &credential_status.state,
                        &input.expected_credential,
                    ) {
                        (CredentialUpdate::Delete {}, CredentialState::Absent, _) => true,
                        (CredentialUpdate::Keep {}, CredentialState::Absent, None) => true,
                        (
                            CredentialUpdate::Keep {},
                            CredentialState::Configured {
                                credential_id,
                                revision,
                                ..
                            },
                            Some(expected),
                        ) => {
                            credential_id == &expected.credential_id
                                && *revision == expected.revision
                        }
                        (
                            CredentialUpdate::Replace { .. },
                            CredentialState::Configured {
                                credential_id,
                                revision,
                                ..
                            },
                            Some(expected),
                        ) => {
                            credential_id == &expected.credential_id
                                && (*revision == expected.revision
                                    || Some(*revision) == expected.revision.checked_add(1))
                        }
                        (
                            CredentialUpdate::Replace { .. },
                            CredentialState::Configured { revision, .. },
                            None,
                        ) => *revision == 1,
                        _ => false,
                    }
            }
            UpdateResult::RevisionConflict {
                expected_revision,
                actual_revision,
            } => {
                *expected_revision == input.expected_policy_revision
                    && actual_revision != expected_revision
            }
            UpdateResult::CredentialStale { expected, actual } => {
                expected == &input.expected_credential
                    && actual != expected
                    && actual
                        .as_ref()
                        .is_none_or(|basis| basis.locator == network_update::locator())
            }
            UpdateResult::ProxyTargetMismatch { expected, actual } => {
                matches!(&input.credential, CredentialUpdate::Replace {expected_target: Some(target), ..} if target == expected && actual != expected)
            }
        };
        if !valid {
            return Err(self.preferences_invalid());
        }
        Ok(result)
    }

    pub async fn test_network_proxy(
        &self,
        input: network_test::Input,
    ) -> Result<network_test::Output, RequestFailure> {
        let wire = serde_json::to_value(input).expect("wire input");
        let input = maka_protocol::network_proxy::decode_input(&wire).map_err(|_| {
            RequestFailure::NotDispatched(ClientError::Protocol(
                "Invalid proxy test settings".into(),
            ))
        })?;
        let value = self
            .request(
                Operation::NetworkProxyTest,
                serde_json::to_value(input).expect("wire input"),
            )
            .await?;
        maka_protocol::network_proxy::decode_output(&value).map_err(|_| self.preferences_invalid())
    }

    pub async fn request_headers(
        &self,
        connection_id: &str,
    ) -> Result<maka_protocol::configuration::headers::RequestHeadersQueryResult, RequestFailure>
    {
        use maka_protocol::configuration::headers::RequestHeadersQueryResult;
        let value = self
            .request(
                Operation::ConnectionRequestHeadersQuery,
                serde_json::json!({"connectionId": connection_id}),
            )
            .await?;
        let result = maka_protocol::request_headers::decode_query_result(&value)
            .map_err(|_| self.preferences_invalid())?;
        if matches!(&result, RequestHeadersQueryResult::Found {basis, names} if basis.connection.connection_id != connection_id || !names.is_empty() && basis.credential.is_none())
        {
            return Err(self.preferences_invalid());
        }
        Ok(result)
    }

    pub async fn replace_request_headers(
        &self,
        input: &maka_protocol::configuration::headers::RequestHeadersReplace,
    ) -> Result<maka_protocol::configuration::headers::RequestHeadersReplaceResult, RequestFailure>
    {
        use maka_protocol::configuration::{
            ConnectionCredentialKind, CredentialLocator,
            headers::RequestHeadersReplaceResult as Result,
        };
        let input = maka_protocol::request_headers::decode_replace(
            &serde_json::to_value(input).expect("wire input"),
        )
        .map_err(|_| {
            RequestFailure::NotDispatched(ClientError::Protocol("Invalid request headers".into()))
        })?;
        let value = self
            .request(
                Operation::ConnectionRequestHeadersReplace,
                serde_json::to_value(&input).expect("wire input"),
            )
            .await?;
        let result = maka_protocol::request_headers::decode_replace_result(&value)
            .map_err(|_| self.preferences_invalid())?;
        let names: std::collections::BTreeSet<_> =
            input.headers.iter().map(|h| h.name.as_str()).collect();
        let locator = CredentialLocator::Connection {
            connection_id: input.expected.connection.connection_id.clone(),
            kind: ConnectionCredentialKind::RequestHeaders,
        };
        let valid = match &result {
            Result::Unchanged {
                names: actual,
                basis,
            } => {
                basis == &input.expected
                    && actual
                        .iter()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                        == names
            }
            Result::Committed {
                names: actual,
                basis,
            } => {
                actual
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>()
                    == names
                    && basis.connection.connection_id == input.expected.connection.connection_id
                    && (basis.connection.revision == input.expected.connection.revision
                        || Some(basis.connection.revision)
                            == input.expected.connection.revision.checked_add(1))
                    && if names.is_empty() {
                        basis.credential.is_none()
                    } else {
                        basis.credential.as_ref().is_some_and(|current| {
                            current.locator == locator
                                && input.expected.credential.as_ref().map_or(
                                    current.revision == 1,
                                    |expected| {
                                        current.credential_id == expected.credential_id
                                            && Some(current.revision)
                                                == expected.revision.checked_add(1)
                                    },
                                )
                        })
                    }
            }
            Result::ConnectionStale { expected, actual } => {
                expected == &input.expected.connection
                    && actual.connection_id == expected.connection_id
                    && actual.revision != expected.revision
            }
            Result::CredentialStale { expected, actual } => {
                expected == &input.expected.credential
                    && actual != expected
                    && actual.as_ref().is_none_or(|basis| basis.locator == locator)
            }
            Result::ConnectionNotFound => true,
        };
        if !valid {
            return Err(self.preferences_invalid());
        }
        Ok(result)
    }

    fn preferences_invalid(&self) -> RequestFailure {
        self.disconnect();
        RequestFailure::Unknown(ClientError::Protocol(
            "Preferences result does not match request".into(),
        ))
    }
}
