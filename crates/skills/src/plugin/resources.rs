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

//! Invocable Skills use the same contribution-bound selectors as other inputs.
use super::{Skills, Snapshot};
use crate::api::{InvocableInput, InvocableItem, InvocableResult, InvocableTarget};
use futures_util::future::BoxFuture;
use maka_plugins::{
    Error,
    input::resources::{self, Context, Query, Resolve},
};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    revision: Option<String>,
    page: Option<String>,
    offset: usize,
    query: String,
}
impl Skills {
    async fn resource_snapshot(&self, context: &Context) -> Result<Snapshot, Error> {
        if !context.tools.contains("Skill") {
            return Ok(Snapshot::empty());
        }
        self.capture(&context.workspace, context.tools.clone())
            .await
            .map_err(|e| Error::Invalid(e.to_string()))
    }
}
fn page(
    snapshot: &Snapshot,
    context: &Context,
    cursor: &Cursor,
) -> Result<(String, Vec<InvocableItem>, Option<String>), Error> {
    let target = InvocableTarget::Session {
        session_id: context.session_id.clone(),
    };
    let input = match (&cursor.revision, &cursor.page) {
        (Some(revision), Some(page)) => InvocableInput::Continue {
            target,
            revision: revision.clone(),
            cursor: page.clone(),
        },
        (_, None) => InvocableInput::Start { target },
        _ => return Err(Error::Invalid("Invalid Skills cursor".into())),
    };
    match snapshot
        .invocable(&input, &context.cwd)
        .map_err(|e| Error::Invalid(e.to_string()))?
    {
        InvocableResult::Page {
            revision,
            items,
            next_cursor,
        } => {
            if cursor.revision.as_ref().is_some_and(|old| old != &revision) {
                return Err(Error::Invalid("Skills changed; start a fresh query".into()));
            }
            Ok((revision, items, next_cursor))
        }
        InvocableResult::RevisionChanged { .. } => {
            Err(Error::Invalid("Skills changed; start a fresh query".into()))
        }
    }
}
impl resources::Provider for Skills {
    fn query(
        &self,
        request: Query,
        context: Context,
    ) -> BoxFuture<'static, Result<resources::Page, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let mut cursor: Cursor = request
                .cursor
                .as_ref()
                .map(|value| serde_json::from_str(value).map_err(|e| Error::Invalid(e.to_string())))
                .transpose()?
                .unwrap_or(Cursor {
                    query: request.query.clone(),
                    ..Default::default()
                });
            if cursor.query != request.query {
                return Err(Error::Invalid(
                    "Skills cursor belongs to another query".into(),
                ));
            }
            let snapshot = this.resource_snapshot(&context).await?;
            let (revision, items, next) = page(&snapshot, &context, &cursor)?;
            let query = request.query.to_lowercase();
            let filtered: Vec<_> = items
                .into_iter()
                .filter(|item| {
                    [&item.name, &item.description, &item.reference]
                        .into_iter()
                        .any(|text| text.to_lowercase().contains(&query))
                })
                .collect();
            if cursor.offset > filtered.len() {
                return Err(Error::Invalid("Invalid Skills cursor offset".into()));
            }
            let items: Vec<_> = filtered
                .iter()
                .skip(cursor.offset)
                .take(request.limit)
                .map(|item| resources::Item {
                    id: item.id.clone(),
                    title: item.name.chars().take(256).collect(),
                    description: Some(item.description.chars().take(2048).collect()),
                })
                .collect();
            cursor.revision = Some(revision);
            cursor.offset += items.len();
            let more = if cursor.offset < filtered.len() {
                true
            } else if let Some(next) = next {
                cursor.page = Some(next);
                cursor.offset = 0;
                true
            } else {
                false
            };
            Ok(resources::Page {
                items,
                next_cursor: if more {
                    Some(
                        serde_json::to_string(&cursor)
                            .map_err(|e| Error::Invalid(e.to_string()))?,
                    )
                } else {
                    None
                },
            })
        })
    }
    fn resolve(
        &self,
        request: Resolve,
        context: Context,
    ) -> BoxFuture<'static, Result<resources::Value, Error>> {
        let this = self.clone();
        Box::pin(async move {
            let snapshot = this.resource_snapshot(&context).await?;
            let mut cursor = Cursor::default();
            loop {
                let (revision, items, next) = page(&snapshot, &context, &cursor)?;
                if let Some(item) = items.into_iter().find(|item| item.id == request.id) {
                    return Ok(resources::Value {
                        selector: item.id,
                        label: item.name.chars().take(256).collect(),
                        quote: None,
                    });
                }
                let Some(next) = next else {
                    return Err(Error::Invalid("This Skill is no longer invocable".into()));
                };
                cursor.revision = Some(revision);
                cursor.page = Some(next);
            }
        })
    }
}
