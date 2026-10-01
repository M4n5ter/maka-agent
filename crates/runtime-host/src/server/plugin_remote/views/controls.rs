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
use maka_plugins::remote::executor_session::{Configure, Create, Session};
use maka_protocol::{
    Operation,
    session::{self, SessionUpdateResult},
};
use maka_runtime::execution::WorkspaceTarget;
pub(super) fn write_error(error: maka_protocol::OperationError) -> Error {
    match error.code {
        maka_protocol::OperationErrorCode::CommitOutcomeUnknown
        | maka_protocol::OperationErrorCode::OutcomeUnknown => Error::OutcomeUnknown(error.message),
        _ => Error::Provider(error.message),
    }
}
fn creation_input(input: &Create) -> Result<session::SessionCreateInput, Error> {
    let mut value = serde_json::json!({"sessionId":input.session_id,"workspace":input.workspace,"executorId":input.executor_id,"executorSettings":input.settings});
    if let Some(name) = &input.name {
        value["name"] = name.clone().into();
    }
    session::decode_session_create_input(&value).map_err(|e| Error::Invalid(e.to_string()))
}
fn projection(item: session::SessionCatalogProjection) -> Session {
    Session {
        session_id: item.id,
        revision: item.revision,
        name: item.name,
        workspace: item.workspace.target,
        executor_id: item.executor_id,
        settings: item.executor_settings.unwrap_or_default(),
    }
}
impl SessionViews {
    pub(super) async fn controlled_creation(
        &self,
        input: Create,
    ) -> Result<Option<Session>, Error> {
        self.session_scope(&input.session_id)?;
        let _lease = self.owner.resource_call().map_err(|_| Error::Retired)?;
        let host = self.host_for(Operation::SessionCatalogQuery).await?;
        let prepared = crate::session::PreparedSession::new(creation_input(&input)?)
            .map_err(|e| Error::Invalid(e.to_string()))?;
        crate::session::require_unmanaged(
            &host.log,
            &input.session_id,
            maka_protocol::OperationErrorCode::OperationConflict,
        )
        .await
        .map_err(write_error)?;
        let record = host
            .log
            .probe_session_create::<SessionConfiguration>(
                &input.session_id,
                &prepared.fingerprint(),
            )
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        Ok(record
            .map(crate::session::catalog_projection)
            .map(projection))
    }

    fn session_scope(&self, id: &str) -> Result<(), Error> {
        maka_runtime::interaction::entity_id(id).map_err(|e| Error::Invalid(e.into()))?;
        if let maka_plugins::composition::Scope::Session(scope) =
            self.owner.identity().map_err(|_| Error::Retired)?.scope
            && scope != id
        {
            return Err(Error::Invalid("Session exceeds contribution scope".into()));
        }
        Ok(())
    }
    pub(super) async fn controlled_session(&self, id: String) -> Result<Option<Session>, Error> {
        self.session_scope(&id)?;
        let _lease = self.owner.resource_call().map_err(|_| Error::Retired)?;
        let host = self.host_for(Operation::SessionCatalogQuery).await?;
        let Some(record) = host
            .log
            .get_session::<SessionConfiguration>(&id)
            .await
            .map_err(|e| Error::Provider(e.to_string()))?
        else {
            return Ok(None);
        };
        crate::session::require_unmanaged(
            &host.log,
            &id,
            maka_protocol::OperationErrorCode::OperationConflict,
        )
        .await
        .map_err(write_error)?;
        if record.archived {
            return Err(Error::Invalid("Session is archived".into()));
        }
        let (executor_id, settings) = match record.configuration.target {
            crate::session::SessionTarget::Executor {
                executor_id,
                settings,
            } => (Some(executor_id), settings),
            _ => (None, Default::default()),
        };
        Ok(Some(Session {
            session_id: id,
            revision: record.revision,
            name: record.configuration.name,
            workspace: record.configuration.workspace.target,
            executor_id,
            settings,
        }))
    }
    pub(super) async fn create_controlled_session(&self, input: Create) -> Result<Session, Error> {
        if !matches!(
            self.owner.identity().map_err(|_| Error::Retired)?.scope,
            maka_plugins::composition::Scope::Profile
        ) {
            return Err(Error::Invalid(
                "Root creation requires a profile contribution".into(),
            ));
        }
        if matches!(input.workspace, WorkspaceTarget::HostPath { .. })
            && self.access != Access::HostPaths
        {
            return Err(Error::Invalid("Endpoint does not allow Host paths".into()));
        }
        let _lease = self.owner.resource_call().map_err(|_| Error::Retired)?;
        let host = self.host_for(Operation::SessionCreate).await?;
        let input = creation_input(&input)?;
        let id = input.session_id.clone();
        let output = super::super::super::sessions::execute(
            &host,
            Operation::SessionCreate,
            &serde_json::to_value(input).map_err(|e| Error::Invalid(e.to_string()))?,
        )
        .await
        .map_err(write_error)?;
        let super::super::super::sessions::Output::Item(item) = output else {
            return Err(Error::OutcomeUnknown("Invalid creation result".into()));
        };
        host.session_catalog
            .publish_session(&id)
            .await
            .map_err(|e| Error::OutcomeUnknown(e.to_string()))?;
        Ok(projection(*item))
    }
    pub(super) async fn configure_controlled_session(
        &self,
        input: Configure,
    ) -> Result<maka_plugins::remote::executor_session::Configured, Error> {
        let id = self
            .session_id
            .as_ref()
            .ok_or_else(|| Error::Invalid("Bind configuration to a Session".into()))?;
        self.session_scope(id)?;
        let _lease = self.owner.resource_call().map_err(|_| Error::Retired)?;
        let host = self.host_for(Operation::SessionConfigurationUpdate).await?;
        let value = serde_json::json!({"sessionId":id,"expectedRevision":input.expected_revision,"patch":{"executorTarget":{"executorId":input.executor_id,"settings":input.settings}}});
        let output = super::super::super::sessions::execute(
            &host,
            Operation::SessionConfigurationUpdate,
            &value,
        )
        .await
        .map_err(write_error)?;
        let super::super::super::sessions::Output::Mutation(outcome) = output else {
            return Err(Error::OutcomeUnknown("Invalid configuration result".into()));
        };
        match outcome {
            SessionUpdateResult::Committed { session } => {
                host.session_catalog
                    .publish_session(id)
                    .await
                    .map_err(|e| Error::OutcomeUnknown(e.to_string()))?;
                Ok(
                    maka_plugins::remote::executor_session::Configured::Committed {
                        revision: session.revision,
                    },
                )
            }
            SessionUpdateResult::RevisionConflict {
                expected_revision,
                actual_revision,
            } => Ok(
                maka_plugins::remote::executor_session::Configured::RevisionConflict {
                    expected_revision,
                    actual_revision,
                },
            ),
        }
    }
}
