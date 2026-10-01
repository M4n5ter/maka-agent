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

use futures_util::future::BoxFuture;
use maka_plugins::{
    composition::Scope,
    contributions::Staged,
    kernel::{Plugin, PluginContext},
    remote::{Endpoint, Handler, key},
};
use serde_json::Value;
use std::sync::Arc;

mod remote;
mod terminal;

pub const ID: &str = "maka.insights";

pub struct Builtin;

impl Plugin for Builtin {
    fn description(&self) -> Option<maka_plugins::kernel::Description> {
        Some(maka_plugins::kernel::Description {
            name: maka_plugins::terminal_ui::Text::localized(
                "Usage insights",
                "使用统计",
                "使用統計",
            ),
            summary: Some(maka_plugins::terminal_ui::Text::localized(
                "Review model usage and costs.",
                "查看模型用量和费用。",
                "查看模型用量與費用。",
            )),
        })
    }

    fn supports_scope(&self, scope: &Scope) -> bool {
        *scope == Scope::Profile
    }
    fn validate(&self, _: &Scope, config: &Value) -> Result<(), maka_plugins::Error> {
        if config.is_null() || config.as_object().is_some_and(|value| value.is_empty()) {
            Ok(())
        } else {
            Err(maka_plugins::Error::Invalid(
                "Insights takes no instance configuration".into(),
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

            let host = context.host.ok_or("Insights requires Host capabilities")?;
            let insights = remote::Insights {
                usage: host.usage,
                pricing: host.pricing,
                store: host.storage,
            };
            terminal::publish(insights.clone(), &identity.package_id, &mut staged)?;
            staged
                .insert(
                    key(&identity.package_id, "request").map_err(message)?,
                    Endpoint::standalone(Handler::Method(Arc::new(insights))),
                )
                .map_err(message)?;
            Ok(staged)
        })
    }
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
