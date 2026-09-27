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

mod binding;
mod file;

use super::{Result, failure, stored};
use crate::server::Host;
use async_compression::tokio::{bufread::GzipDecoder, write::GzipEncoder};
use maka_event_log::{
    StoreError,
    bundle::{BundleError, StagedBundle},
};
use maka_protocol::{OperationErrorCode as Code, session::bundle as api};
use std::time::Duration;
use tokio::{
    io::{AsyncWriteExt, BufReader},
    sync::Semaphore,
};

static TRANSFERS: Semaphore = Semaphore::const_new(2);
const READ_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) async fn preview(host: &Host, input: api::Preview) -> Result<api::Previewed> {
    let inventory = host
        .log
        .preview_bundle(&input.session_id)
        .await
        .map_err(error)?;
    Ok(api::Previewed {
        session_count: inventory.sessions.len() as u64,
        subtree_digest: inventory.subtree_digest,
    })
}

pub(super) async fn export(host: &Host, input: api::Export) -> Result<api::Exported> {
    let _permit = TRANSFERS.try_acquire().map_err(|_| {
        failure(
            Code::OperationConflict,
            "Two Session transfers are already active",
        )
    })?;
    let inventory = host
        .log
        .preview_bundle(&input.session_id)
        .await
        .map_err(error)?;
    let expected = match input.expected_subtree_digest {
        Some(digest) => digest,
        None if inventory.sessions.len() == 1 => inventory.subtree_digest,
        None => {
            return Err(failure(
                Code::CandidateSetStale,
                "Confirm the complete Session subtree before exporting",
            ));
        }
    };
    let output = GzipEncoder::new(file::Output::open(host, &input.destination).await?);
    let (mut output, summary) = host
        .log
        .export_bundle(&input.session_id, &expected, output)
        .await
        .map_err(error)?;
    output
        .shutdown()
        .await
        .map_err(|e| failure(Code::PersistenceFailed, &e.to_string()))?;
    let compressed_bytes = output.into_inner().publish().await?;
    Ok(api::Exported {
        session_count: summary.inventory.sessions.len() as u64,
        compressed_bytes,
    })
}

pub(super) async fn import(host: &Host, input: api::Import) -> Result<api::Imported> {
    let _permit = TRANSFERS.try_acquire().map_err(|_| {
        failure(
            Code::OperationConflict,
            "Two Session transfers are already active",
        )
    })?;
    let mut staged = stage(host, &input.source).await?;
    validate(&mut staged).await?;
    let _admission = host.executions.lock_admission().await;
    if host.draining.is_cancelled() {
        return Err(failure(Code::HostDraining, "Host is draining"));
    }
    let destination = super::workspace::resolve(host, &input.workspace).await?;
    let actual = import_identity(&mut staged, &input.workspace, destination).await?;
    if actual != input.expected {
        staged.close().await.map_err(error)?;
        return Err(failure(
            Code::CandidateSetStale,
            "Bundle content or destination binding differs from its preview",
        ));
    }
    let binding = &actual.binding_digest;
    // Another transfer may have committed while we validated or waited for
    // admission. Its accepted binding wins over defaults that changed since.
    if let Some(receipt) = host
        .log
        .bundle_import_receipt(&actual.bundle_digest, binding)
        .await
        .map_err(stored)?
    {
        staged.close().await.map_err(error)?;
        return Ok(imported(receipt, actual.artifact_files));
    }
    // Project mutations share admission. Reuse this exact resolved projection
    // through publication instead of resolving the user's locator a second time.
    let configurations = binding::resolve(host, &mut staged, actual.resolved_workspace).await?;
    let result = host
        .log
        .import_bundle(staged, binding, configurations)
        .await;
    let receipt = match result {
        Ok(receipt) => receipt,
        // The importer can commit before staging cleanup fails. Settle that
        // boundary from its atomic receipt instead of reporting a rejected write.
        Err(error) => match host
            .log
            .bundle_import_receipt(&actual.bundle_digest, binding)
            .await
        {
            Ok(Some(receipt)) => receipt,
            Ok(None) => return Err(source_error(error)),
            Err(readback) => {
                return Err(failure(Code::CommitOutcomeUnknown, &readback.to_string()));
            }
        },
    };
    // One unscoped notice invalidates the catalog, including Sessions without events.
    host.session_catalog
        .publish_all()
        .await
        .map_err(|e| failure(Code::CommitOutcomeUnknown, &e.to_string()))?;
    Ok(imported(receipt, actual.artifact_files))
}

pub(super) async fn import_preview(
    host: &Host,
    input: api::ImportPreview,
) -> Result<api::ImportPreviewed> {
    let _permit = TRANSFERS.try_acquire().map_err(|_| {
        failure(
            Code::OperationConflict,
            "Two Session transfers are already active",
        )
    })?;
    let mut staged = stage(host, &input.source).await?;
    validate(&mut staged).await?;
    let _admission = host.executions.lock_admission().await;
    let destination = super::workspace::resolve(host, &input.workspace).await?;
    let preview = import_identity(&mut staged, &input.workspace, destination).await?;
    staged.close().await.map_err(error)?;
    Ok(preview)
}

/// Recovery reads the original atomic receipt, even if the source path, current
/// defaults, or imported Sessions have since changed. It never starts an import.
pub(super) async fn import_query(
    host: &Host,
    input: api::ImportQuery,
) -> Result<api::ImportQueried> {
    let receipt = host
        .log
        .bundle_import_receipt(&input.bundle_digest, &input.binding_digest)
        .await
        .map_err(stored)?;
    Ok(api::ImportQueried {
        receipt: receipt.map(|receipt| api::ImportReceipt {
            root_session_id: receipt.root_session_id,
            session_ids: receipt.session_ids,
        }),
    })
}

async fn stage(host: &Host, source: &str) -> Result<StagedBundle> {
    let file = file::input(host, source).await?;
    let mut reader = GzipDecoder::new(BufReader::new(file));
    // The bundle reader checks decoded EOF; concatenated members cannot hide
    // trailing content behind an otherwise valid first gzip stream.
    reader.multiple_members(true);
    tokio::time::timeout(READ_TIMEOUT, StagedBundle::read(reader))
        .await
        .map_err(|_| failure(Code::SourceUnreadable, "Bundle read deadline exceeded"))?
        .map_err(source_error)
}

async fn validate(staged: &mut StagedBundle) -> Result<()> {
    tokio::time::timeout(READ_TIMEOUT, staged.validate_history())
        .await
        .map_err(|_| {
            failure(
                Code::SourceUnreadable,
                "Bundle validation deadline exceeded",
            )
        })?
        .map_err(source_error)
}

async fn import_identity(
    staged: &mut StagedBundle,
    workspace: &maka_protocol::session::WorkspaceTarget,
    resolved_workspace: maka_protocol::session::WorkspaceProjection,
) -> Result<api::ImportPreviewed> {
    Ok(api::ImportPreviewed {
        artifact_files: staged.artifact_count().await.map_err(error)?,
        session_count: staged.summary().inventory.sessions.len() as u64,
        bundle_digest: staged.summary().digest.clone(),
        resolved_workspace,
        binding_digest: maka_runtime::artifact::content_digest(
            &serde_json::to_vec(&("session-bundle.import.v1", workspace))
                .map_err(|e| failure(Code::InvalidRequest, &e.to_string()))?,
        ),
    })
}

fn imported(receipt: maka_event_log::bundle::ImportReceipt, artifact_files: u64) -> api::Imported {
    api::Imported {
        session_count: receipt.session_ids.len() as u64,
        artifact_files,
        root_session_id: receipt.root_session_id,
        session_ids: receipt.session_ids,
    }
}

fn source_error(error: BundleError) -> maka_protocol::OperationError {
    match error {
        BundleError::Store(StoreError::Io(_) | StoreError::Json(_)) => {
            failure(Code::SourceUnreadable, &error.to_string())
        }
        other => self::error(other),
    }
}

fn error(error: BundleError) -> maka_protocol::OperationError {
    match error {
        BundleError::CandidateSetStale => failure(Code::CandidateSetStale, &error.to_string()),
        BundleError::TooManySessions => failure(Code::InvalidRequest, &error.to_string()),
        BundleError::Store(
            error @ (StoreError::InvalidTransition(_) | StoreError::PrefixTooLarge),
        ) => failure(Code::SourceUnreadable, &error.to_string()),
        BundleError::Store(error) => stored(error),
    }
}
