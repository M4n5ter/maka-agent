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

use crate::{Client, RequestFailure};
use maka_protocol::{Operation, ProtocolError, session::workspace_context as api};
impl Client {
    pub async fn query_session_workspace(
        &self,
        input: api::Query,
    ) -> Result<api::Page, RequestFailure> {
        let value = self
            .request(
                Operation::SessionWorkspaceQuery,
                serde_json::to_value(&input).expect("wire input"),
            )
            .await?;
        let output = api::decode_page(&value).map_err(|error| self.invalid_session(error))?;
        if output.basis.root_id != self.identity.root_id
            || output.basis.session_id != input.session_id
            || output.directory != input.directory
            || output.filter != input.filter
            || input.cursor.as_ref().is_some_and(|cursor| {
                cursor.basis != output.basis
                    || cursor.revision != output.revision
                    || output
                        .entries
                        .first()
                        .is_some_and(|row| row.path <= cursor.after)
            })
        {
            return Err(self.invalid_session(ProtocolError::invalid(
                "Workspace page differs from its query",
            )));
        }
        Ok(output)
    }
    pub async fn capture_session_workspace(
        &self,
        input: api::Capture,
    ) -> Result<api::Captured, RequestFailure> {
        let value = self
            .request(
                Operation::SessionWorkspaceCapture,
                serde_json::to_value(&input).expect("wire input"),
            )
            .await?;
        let output = api::decode_captured(&value).map_err(|error| self.invalid_session(error))?;
        if output.basis != input.basis
            || output.basis.root_id != self.identity.root_id
            || output.path != input.path
            || output.kind != input.kind
        {
            return Err(self.invalid_session(ProtocolError::invalid(
                "Workspace capture differs from its selection",
            )));
        }
        Ok(output)
    }
}
