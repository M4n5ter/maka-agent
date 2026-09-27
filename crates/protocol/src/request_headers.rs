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

use crate::{ProtocolError, Result};
use maka_runtime::configuration::{
    ConnectionCredentialKind, CredentialLocator, headers::*, validation,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone())
        .map_err(|_| ProtocolError::invalid("Invalid request headers payload"))
}

pub fn decode_query(value: &Value) -> Result<RequestHeadersQuery> {
    let input: RequestHeadersQuery = decode(value)?;
    validation::entity_id(&input.connection_id).map_err(ProtocolError::invalid)?;
    Ok(input)
}

pub fn decode_replace(value: &Value) -> Result<RequestHeadersReplace> {
    let mut input: RequestHeadersReplace = decode(value)?;
    input.normalize().map_err(ProtocolError::invalid)?;
    Ok(input)
}

fn normalize_names(names: &mut Vec<String>) -> Result<()> {
    let mut updates = names
        .iter()
        .map(|name| RequestHeaderUpdate {
            name: name.clone(),
            value: None,
        })
        .collect::<Vec<_>>();
    normalize_updates(&mut updates).map_err(ProtocolError::invalid)?;
    *names = updates.into_iter().map(|update| update.name).collect();
    Ok(())
}

pub fn decode_query_result(value: &Value) -> Result<RequestHeadersQueryResult> {
    result_fields(value)?;
    let mut result = decode(value)?;
    if let RequestHeadersQueryResult::Found { names, basis } = &mut result {
        normalize_names(names)?;
        basis.validate().map_err(ProtocolError::invalid)?;
    }
    Ok(result)
}

pub fn decode_replace_result(value: &Value) -> Result<RequestHeadersReplaceResult> {
    result_fields(value)?;
    let mut result = decode(value)?;
    match &mut result {
        RequestHeadersReplaceResult::Committed { names, basis }
        | RequestHeadersReplaceResult::Unchanged { names, basis } => {
            normalize_names(names)?;
            basis.validate().map_err(ProtocolError::invalid)?;
        }
        RequestHeadersReplaceResult::ConnectionNotFound => {}
        RequestHeadersReplaceResult::ConnectionStale { expected, actual } => {
            validation::basis(expected).map_err(ProtocolError::invalid)?;
            validation::basis(actual).map_err(ProtocolError::invalid)?;
            if expected.connection_id != actual.connection_id {
                return Err(ProtocolError::invalid(
                    "Request headers connection mismatch",
                ));
            }
        }
        RequestHeadersReplaceResult::CredentialStale { expected, actual } => {
            for basis in [expected.as_ref(), actual.as_ref()].into_iter().flatten() {
                validation::credential_basis(basis).map_err(ProtocolError::invalid)?;
                if !matches!(
                    basis.locator,
                    CredentialLocator::Connection {
                        kind: ConnectionCredentialKind::RequestHeaders,
                        ..
                    }
                ) {
                    return Err(ProtocolError::invalid("Invalid request headers credential"));
                }
            }
            if let (Some(expected), Some(actual)) = (expected, actual)
                && expected.locator != actual.locator
            {
                return Err(ProtocolError::invalid(
                    "Request headers credential mismatch",
                ));
            }
        }
    }
    Ok(result)
}

fn result_fields(value: &Value) -> Result<()> {
    let record = crate::codec::record(value, "request headers result")?;
    crate::codec::exact(
        record,
        match value["kind"].as_str() {
            Some("connection_not_found") => &["kind"],
            Some("connection_stale" | "credential_stale") => &["kind", "expected", "actual"],
            _ => &["kind", "names", "basis"],
        },
    )
}
