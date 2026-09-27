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

mod files;
use super::{Result, failure, stored};
use crate::{server::Host, session::SessionConfiguration};
use maka_protocol::{OperationErrorCode as Code, session::workspace_context as api};
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::Semaphore;

static READS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(4)));

pub(super) async fn query(host: &Host, input: api::Query) -> Result<api::Page> {
    let session = input.session_id.clone();
    let output = read(host, &session, move |scope| scope.query(input)).await?;
    revalidate(host, &output.basis).await?;
    Ok(output)
}

pub(super) async fn capture(host: &Host, input: api::Capture) -> Result<api::Captured> {
    revalidate(host, &input.basis).await?;
    let session = input.basis.session_id.clone();
    let output = read(host, &session, move |scope| scope.capture(input)).await?;
    revalidate(host, &output.basis).await?;
    Ok(output)
}

async fn configuration(host: &Host, session: &str) -> Result<SessionConfiguration> {
    let record = host
        .log
        .get_session::<SessionConfiguration>(session)
        .await
        .map_err(stored)?
        .ok_or_else(|| failure(Code::NotFound, "Session was not found"))?;
    if record.archived {
        return Err(failure(Code::OperationConflict, "Session is archived"));
    }
    host.executions.validate_workspace(&record.configuration)?;
    Ok(record.configuration)
}

async fn revalidate(host: &Host, basis: &api::Basis) -> Result<()> {
    let current = configuration(host, &basis.session_id).await?;
    if basis.root_id != host.root_id()
        || basis.boundary_revision != current.boundary_revision
        || basis.workspace != current.workspace
    {
        return Err(stale());
    }
    Ok(())
}

async fn read<T: Send + 'static>(
    host: &Host,
    session: &str,
    work: impl FnOnce(files::Scope) -> Result<T> + Send + 'static,
) -> Result<T> {
    let configuration = configuration(host, session).await?;
    let permit = READS
        .clone()
        .try_acquire_owned()
        .map_err(|_| failure(Code::OperationConflict, "Workspace read capacity is busy"))?;
    let cancellation = host.draining.child_token();
    let _cancel = cancellation.clone().drop_guard();
    let root = host.root_id().to_owned();
    let state = host.root.canonical_path().to_owned();
    let control = host.control_directory().to_owned();
    let session = session.to_owned();
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(files::Scope::open(
            root,
            session,
            configuration,
            state,
            control,
            cancellation,
        )?)
    });
    tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .map_err(|_| {
            failure(
                Code::OperationUnavailable,
                "Workspace read deadline exceeded",
            )
        })?
        .map_err(|error| failure(Code::InternalFailure, &error.to_string()))?
}

fn stale() -> maka_protocol::OperationError {
    failure(
        Code::CandidateSetStale,
        "Workspace or candidate inventory changed",
    )
}
fn unreadable(error: impl std::fmt::Display) -> maka_protocol::OperationError {
    failure(Code::SourceUnreadable, &error.to_string())
}
