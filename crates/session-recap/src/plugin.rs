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

use crate::recap::Recaps;
use futures_util::future::BoxFuture;
use maka_plugins::{
    composition::Scope,
    contributions::Staged,
    kernel::{Plugin, PluginContext},
};
use serde_json::Value;
use std::sync::Arc;
mod remote;
mod terminal;
pub const ID: &str = "maka.session-recap";
pub struct Builtin;

impl Plugin for Builtin {
    fn description(&self) -> Option<maka_plugins::kernel::Description> {
        Some(maka_plugins::kernel::Description {
            name: maka_plugins::terminal_ui::Text::localized(
                "Conversation recap",
                "会话回顾",
                "對話回顧",
            ),
            summary: Some(maka_plugins::terminal_ui::Text::localized(
                "Create a summary of a previous conversation.",
                "为历史会话生成摘要。",
                "為歷史對話產生摘要。",
            )),
        })
    }

    fn supports_scope(&self, scope: &Scope) -> bool {
        *scope == Scope::Profile
    }
    fn validate(&self, _: &Scope, value: &Value) -> Result<(), maka_plugins::Error> {
        if value.is_null() || value.as_object().is_some_and(|v| v.is_empty()) {
            Ok(())
        } else {
            Err(maka_plugins::Error::Invalid(
                "Session recap takes no instance configuration".into(),
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

            let host = context
                .host
                .ok_or("Session recap requires Host capabilities")?;
            let backend = Arc::new(Recaps {
                store: host.storage,
                history: host.history,
                models: host.models,
                preferences: host.preferences,
            });
            remote::publish(backend.clone(), &identity.package_id, &mut staged)?;
            terminal::publish(backend, &identity.package_id, &mut staged)?;

            Ok(staged)
        })
    }
}
fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}
