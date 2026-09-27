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
use maka_protocol::{Operation, ProtocolError, session::bundle};

impl Client {
    pub async fn preview_session_bundle(
        &self,
        input: bundle::Preview,
    ) -> Result<bundle::Previewed, RequestFailure> {
        let value = self
            .request(
                Operation::SessionBundlePreview,
                serde_json::to_value(input).expect("wire input"),
            )
            .await?;
        bundle::decode_previewed(&value).map_err(|error| self.invalid_session(error))
    }

    /// Export names a destination on the Host. It is never a client download.
    pub async fn export_session_bundle(
        &self,
        input: bundle::Export,
    ) -> Result<bundle::Exported, RequestFailure> {
        let value = self
            .request(
                Operation::SessionBundleExport,
                serde_json::to_value(input).expect("wire input"),
            )
            .await?;
        bundle::decode_exported(&value).map_err(|error| self.invalid_session(error))
    }

    pub async fn preview_session_bundle_import(
        &self,
        input: bundle::ImportPreview,
    ) -> Result<bundle::ImportPreviewed, RequestFailure> {
        let value = self
            .request(
                Operation::SessionBundleImportPreview,
                serde_json::to_value(input).expect("wire input"),
            )
            .await?;
        bundle::decode_import_previewed(&value).map_err(|error| self.invalid_session(error))
    }

    /// Import names a source on the Host and binds its content to the user's
    /// preview. Unknown writes are recovered by querying their original receipt.
    pub async fn import_session_bundle(
        &self,
        input: bundle::Import,
    ) -> Result<bundle::Imported, RequestFailure> {
        let value = self
            .request(
                Operation::SessionBundleImport,
                serde_json::to_value(&input).expect("wire input"),
            )
            .await?;
        let output =
            bundle::decode_imported(&value).map_err(|error| self.invalid_session(error))?;
        if output.session_count != input.expected.session_count
            || output.artifact_files != input.expected.artifact_files
        {
            return Err(self.invalid_session(ProtocolError::invalid(
                "Bundle import receipt changed its preview",
            )));
        }
        Ok(output)
    }

    /// This read does not reopen the source path or repeat import admission.
    /// Missing does not prove that an earlier import was never admitted.
    pub async fn query_session_bundle_import(
        &self,
        input: bundle::ImportQuery,
    ) -> Result<bundle::ImportQueried, RequestFailure> {
        let value = self
            .request(
                Operation::SessionBundleImportQuery,
                serde_json::to_value(input).expect("wire input"),
            )
            .await?;
        bundle::decode_import_queried(&value).map_err(|error| self.invalid_session(error))
    }
}
