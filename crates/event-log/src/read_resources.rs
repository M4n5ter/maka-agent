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

use crate::{EventLog, StoreError};
use maka_runtime::read::ResourceAddress;
use sqlx::Connection;

impl EventLog {
    /// Resolve a locator in the reader's visible namespace, never across Sessions.
    /// Content reads retain their own evidence and authorization checks.
    pub async fn resolve_read_resource(
        &self,
        session: &str,
        address: ResourceAddress,
    ) -> Result<Option<ResourceAddress>, StoreError> {
        self.validate_root()?;
        crate::sessions::validate_id(session)?;
        address
            .canonical_path()
            .map_err(|error| StoreError::InvalidTransition(error.into()))?;
        let (query, column) = match &address {
            ResourceAddress::Attachment(_) => (
                "SELECT id FROM artifacts WHERE session_id=?1
                 AND json_extract(record_json,'$.source')='user_upload'",
                "id",
            ),
            ResourceAddress::Task(_) => (
                "SELECT id FROM shell_runs WHERE session_id=?1
                 AND json_extract(record_json,'$.visibility')='model'",
                "id",
            ),
            ResourceAddress::ToolResult(_) => (
                "SELECT t.event_id FROM session_history_events t
                 JOIN runtime_events d ON d.invocation_id=t.invocation_id
                   AND d.operation_id=t.operation_id AND d.kind='tool_dispatched'
                 WHERE t.owner_session_id=?1 AND t.kind='tool_settled'
                   AND json_extract(d.event_json,'$.fact.call.origin.kind')='provider'
                   AND json_extract(t.event_json,'$.fact.outcome.kind') IN ('succeeded','failed')",
                "t.event_id",
            ),
        };
        let session = session.to_owned();
        self.connection
            .run(move |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await?;
                    // GLOB is case sensitive. Escape its syntax so IDs are literal prefixes.
                    let mut literal = String::new();
                    for character in address.id().chars() {
                        match character {
                            '*' => literal.push_str("[*]"),
                            '?' => literal.push_str("[?]"),
                            '[' => literal.push_str("[[]"),
                            _ => literal.push(character),
                        }
                    }
                    for (operator, pattern) in [
                        ("=", address.id().to_owned()),
                        ("GLOB", format!("{literal}*")),
                    ] {
                        // SQLite's GLOB terminates at NUL; never broaden such a locator.
                        if operator == "GLOB" && address.id().contains('\0') {
                            return Ok(None);
                        }
                        let sql =
                            format!("{query} AND {column} {operator} ?2 ORDER BY {column} LIMIT 2");
                        let matches: Vec<String> =
                            sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                                .bind(&session)
                                .bind(pattern)
                                .fetch_all(&mut *tx)
                                .await?;
                        match matches.len() {
                            0 => {}
                            1 => {
                                return Ok(Some(
                                    address.with_id(matches.into_iter().next().unwrap()),
                                ));
                            }
                            _ => return Err(StoreError::AmbiguousReadResource),
                        }
                    }
                    Ok(None)
                })
            })
            .await
    }
}
