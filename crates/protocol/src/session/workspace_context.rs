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

//! Session-bound discovery and immutable excerpts, never filesystem grants.
use super::WorkspaceProjection;
use crate::{
    OperationErrorCode, ProtocolError, Result,
    turn::{DirectoryReference, QuoteRef},
};
use maka_runtime::execution::DirectoryIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PAGE_BYTES: usize = 48 * 1024;
pub const CAPTURE_BYTES: usize = 24 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Basis {
    pub root_id: String,
    pub session_id: String,
    pub boundary_revision: u64,
    pub workspace: WorkspaceProjection,
    pub directory_identity: DirectoryIdentity,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cursor {
    pub basis: Basis,
    pub revision: String,
    pub after: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub session_id: String,
    pub directory: String,
    pub filter: String,
    pub cursor: Option<Cursor>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Directory,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub basis: Basis,
    pub directory: String,
    pub filter: String,
    pub revision: String,
    pub entries: Vec<Entry>,
    pub next_cursor: Option<Cursor>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capture {
    pub basis: Basis,
    pub path: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Captured {
    pub basis: Basis,
    pub path: String,
    pub kind: Kind,
    pub quote: QuoteRef,
    pub content_digest: String,
    pub truncated: bool,
    pub directory_reference: Option<DirectoryReference>,
}

pub fn decode_query(value: &Value) -> Result<Query> {
    let input: Query = decode(value)?;
    super::validation::entity(&input.session_id)?;
    relative(&input.directory, true)?;
    filter(&input.filter)?;
    if let Some(cursor) = &input.cursor {
        validate_basis(&cursor.basis)?;
        digest(&cursor.revision)?;
        relative(&cursor.after, false)?;
        ensure(
            cursor.basis.session_id == input.session_id,
            "Cursor Session differs",
        )?;
    }
    Ok(input)
}
pub fn decode_page(value: &Value) -> Result<Page> {
    let output: Page = decode(value)?;
    validate_basis(&output.basis)?;
    relative(&output.directory, true)?;
    filter(&output.filter)?;
    digest(&output.revision)?;
    let mut previous = None;
    for entry in &output.entries {
        relative(&entry.path, false)?;
        ensure(
            entry
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&output.filter.to_lowercase()),
            "Candidate differs from its filter",
        )?;
        ensure(
            entry.path.rsplit_once('/').map_or("", |(parent, _)| parent) == output.directory,
            "Candidate is outside its directory",
        )?;
        ensure(
            previous.is_none_or(|path: &str| path < entry.path.as_str()),
            "Candidates are not sorted",
        )?;
        previous = Some(entry.path.as_str());
    }
    if let Some(cursor) = &output.next_cursor {
        ensure(
            cursor.basis == output.basis
                && cursor.revision == output.revision
                && Some(cursor.after.as_str()) == previous,
            "Invalid workspace continuation",
        )?;
    }
    ensure(
        serde_json::to_vec(value).map_err(invalid)?.len() <= PAGE_BYTES,
        "Workspace page exceeds byte limit",
    )?;
    Ok(output)
}
pub fn decode_capture(value: &Value) -> Result<Capture> {
    let input: Capture = decode(value)?;
    validate_basis(&input.basis)?;
    relative(&input.path, input.kind == Kind::Directory)?;
    Ok(input)
}
pub fn decode_captured(value: &Value) -> Result<Captured> {
    let output: Captured = decode(value)?;
    validate_basis(&output.basis)?;
    relative(&output.path, output.kind == Kind::Directory)?;
    digest(&output.content_digest)?;
    ensure(
        maka_runtime::artifact::content_digest(output.quote.text.as_bytes())
            == output.content_digest,
        "Captured excerpt digest differs",
    )?;
    let mut content = crate::turn::MessageContent {
        text: String::new(),
        display_text: None,
        attachments: None,
        inline_references: None,
        quotes: Some(vec![output.quote.clone()]),
        directory_references: output
            .directory_reference
            .clone()
            .map(|reference| vec![reference]),
    };
    content.validate_admission(false)?;
    ensure(
        output.quote.source.is_none() && output.quote.source_turn_id.is_none(),
        "File capture is not Session history",
    )?;
    ensure(
        (output.kind == Kind::Directory) == output.directory_reference.is_some(),
        "Capture kind differs",
    )?;
    if let Some(reference) = &output.directory_reference {
        ensure(
            reference.host_id == output.basis.root_id,
            "Directory belongs to another Host",
        )?;
        ensure(
            reference.path == directory_path(&output.basis.workspace.host_cwd, &output.path),
            "Directory path differs from captured scope",
        )?;
    }
    Ok(output)
}
pub fn validate_basis(basis: &Basis) -> Result<()> {
    super::validation::entity(&basis.root_id)?;
    super::validation::entity(&basis.session_id)?;
    ensure(
        basis.boundary_revision <= 9_007_199_254_740_991,
        "Invalid workspace boundary",
    )?;
    let _: WorkspaceProjection =
        super::validation::decode(&serde_json::to_value(&basis.workspace).map_err(invalid)?)?;
    Ok(())
}
pub fn relative(value: &str, root: bool) -> Result<()> {
    ensure(
        (root && value.is_empty())
            || (value.len() <= 4096
                && maka_plugins::filesystem::entries::validate_path(value).is_ok()
                && !value.chars().any(char::is_control)),
        "Expected a relative workspace path",
    )
}
pub fn directory_path(cwd: &str, relative: &str) -> String {
    if relative.is_empty() {
        cwd.to_owned()
    } else {
        format!("{}/{relative}", cwd.trim_end_matches(['/', '\\']))
    }
}
fn filter(value: &str) -> Result<()> {
    ensure(
        value.len() <= 512 && !value.chars().any(char::is_control),
        "Invalid workspace filter",
    )
}
fn digest(value: &str) -> Result<()> {
    ensure(
        maka_runtime::archive::valid_projection_digest(value),
        "Invalid workspace digest",
    )
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(invalid)
}
fn invalid(error: impl std::fmt::Display) -> ProtocolError {
    ProtocolError::invalid(error.to_string())
}
fn ensure(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::invalid(message))
    }
}

pub const ERRORS: &[OperationErrorCode] = &[
    OperationErrorCode::HostNotReady,
    OperationErrorCode::HostDraining,
    OperationErrorCode::InvalidRequest,
    OperationErrorCode::NotFound,
    OperationErrorCode::OperationUnavailable,
    OperationErrorCode::OperationConflict,
    OperationErrorCode::CandidateSetStale,
    OperationErrorCode::SourceUnreadable,
    OperationErrorCode::PersistenceFailed,
    OperationErrorCode::InternalFailure,
];

#[cfg(test)]
mod tests;
