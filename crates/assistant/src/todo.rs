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

//! Session checklists are plugin data, not execution evidence or a work queue.
mod remote;
mod terminal;
mod tools;

use futures_util::future::BoxFuture;
use maka_plugins::{
    composition::Scope,
    contributions::Staged,
    kernel::{Plugin, PluginContext},
    storage::{Data, Mutation, Store, StoreError},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::watch;

pub const ID: &str = "maka.todo";
const MAX_ITEMS: usize = 200;
const MAX_CONTENT_CHARS: usize = 200;

pub struct Builtin;

impl Plugin for Builtin {
    fn description(&self) -> Option<maka_plugins::kernel::Description> {
        Some(maka_plugins::kernel::Description {
            name: maka_plugins::terminal_ui::Text::localized("Tasks", "待办事项", "待辦事項"),
            summary: Some(maka_plugins::terminal_ui::Text::localized(
                "Keep track of work in the current conversation.",
                "跟踪当前会话中的待办事项。",
                "追蹤目前對話中的待辦事項。",
            )),
        })
    }

    fn supports_scope(&self, scope: &Scope) -> bool {
        *scope == Scope::Profile
    }
    fn validate(&self, _: &Scope, config: &Value) -> Result<(), maka_plugins::Error> {
        if config.is_null() || config.as_object().is_some_and(|object| object.is_empty()) {
            Ok(())
        } else {
            Err(maka_plugins::Error::Invalid(
                "Todo takes no instance configuration".into(),
            ))
        }
    }
    fn activate(
        &self,
        context: PluginContext,
        _config: Value,
    ) -> BoxFuture<'static, Result<Staged, String>> {
        Box::pin(async move {
            let identity = context.lifecycle.identity().map_err(message)?;
            let mut staged = Staged::default();

            let host = context.host.ok_or("Todo requires Host storage")?;
            let repository = Arc::new(Repository {
                store: host.storage,
                changed: watch::channel(()).0,
            });
            tools::publish(repository.clone(), &mut staged)?;
            terminal::publish(repository.clone(), &identity.package_id, &mut staged)?;
            remote::publish(repository, &identity.package_id, &mut staged)?;

            Ok(staged)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Item {
    #[schemars(length(min = 1, max = MAX_CONTENT_CHARS))]
    content: String,
    status: Status,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    InProgress,
    Completed,
}
impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    items: Vec<Item>,
}
impl Document {
    fn normalize(mut items: Vec<Item>) -> Result<Self, String> {
        if items.len() > MAX_ITEMS {
            return Err("Todo supports at most 200 items".into());
        }
        for item in &mut items {
            item.content = item
                .content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if item.content.is_empty()
                || item.content.chars().count() > MAX_CONTENT_CHARS
                || item.content.chars().any(char::is_control)
            {
                return Err(
                    "Todo content must contain 1–200 characters without control codes".into(),
                );
            }
        }
        Ok(Self { items })
    }
    fn render(&self) -> String {
        if self.items.is_empty() {
            return "Todo list is empty.".into();
        }
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                // Quoted text stays data; no secret redaction or invented XML framing.
                format!(
                    "{}. [{}] {}",
                    index + 1,
                    item.status.label(),
                    serde_json::to_string(&item.content).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Snapshot {
    revision: Option<u64>,
    document: Document,
}
struct Repository {
    store: Arc<dyn Store>,
    changed: watch::Sender<()>,
}
impl Repository {
    async fn read(&self, session: &str) -> Result<Snapshot, StoreError> {
        let record = self.store.read(format!("session/{session}")).await?;
        let revision = record.as_ref().map(|record| record.revision);
        let document = match record.and_then(|record| record.data.value().cloned()) {
            Some(value) => serde_json::from_value(value)
                .map_err(|error| StoreError::Unavailable(error.to_string()))?,
            None => Document::default(),
        };
        Ok(Snapshot { revision, document })
    }
    async fn replace(
        &self,
        session: &str,
        expected: Option<u64>,
        document: &Document,
    ) -> Result<(), StoreError> {
        self.store
            .batch(vec![Mutation {
                key: format!("session/{session}"),
                expected_revision: expected,
                data: Data::Present(
                    serde_json::to_value(document)
                        .map_err(|error| StoreError::Unavailable(error.to_string()))?,
                ),
            }])
            .await?;
        self.changed.send_replace(());
        Ok(())
    }
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
