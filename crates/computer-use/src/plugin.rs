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

use crate::definitions;
use futures_util::future::BoxFuture;
use maka_plugins::{
    composition::Scope,
    contributions::Staged,
    kernel::{Plugin, PluginContext},
};
use maka_runtime::{
    tool_call::ToolRejection,
    tool_output::ToolOutput,
    tools::{
        PreparationFuture, PreparedEffect, ToolCallContext, ToolHandler, ToolNesting, ToolPreparer,
        ToolRegistration, ToolSemantics,
    },
};
use maka_tool_catalog::plugins::PluginTool;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct Builtin;

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    browsers: Vec<maka_plugins::computer::BrowserConnection>,
    #[serde(default)]
    desktop: maka_plugins::computer::Desktop,
}

impl Plugin for Builtin {
    fn supports_scope(&self, scope: &Scope) -> bool {
        matches!(scope, Scope::Profile | Scope::Session(_))
    }
    fn validate(&self, _: &Scope, config: &Value) -> Result<(), maka_plugins::Error> {
        let settings: Settings = if config.is_null() {
            Settings::default()
        } else {
            serde_json::from_value(config.clone())
                .map_err(|e| maka_plugins::Error::Invalid(e.to_string()))?
        };
        crate::browser::validate_connections(&settings.browsers)
            .map_err(|e| maka_plugins::Error::Invalid(e.to_string()))
    }
    fn activate(
        &self,
        context: PluginContext,
        config: Value,
    ) -> BoxFuture<'static, Result<Staged, String>> {
        Box::pin(async move {
            let host = context
                .host
                .ok_or("Computer Use requires Host capabilities")?;
            let settings: Settings = if config.is_null() {
                Settings::default()
            } else {
                serde_json::from_value(config).map_err(|e| e.to_string())?
            };
            let handler = Arc::new(Tools(host.computer, settings.browsers, settings.desktop));
            let mut staged = Staged::default();
            for definition in definitions() {
                let name = definition.name.clone();
                let tool = PluginTool::new(ToolRegistration {
                    definition,
                    handler: ToolHandler::Prepared(handler.clone()),
                    nesting: ToolNesting::DirectOnly,
                    semantics: ToolSemantics::ExclusiveStep,
                })
                .map_err(|error| error.to_string())?;
                staged
                    .insert(name, tool)
                    .map_err(|error| error.to_string())?;
            }
            Ok(staged)
        })
    }
}

struct Tools(
    Arc<dyn maka_plugins::computer::Computer>,
    Vec<maka_plugins::computer::BrowserConnection>,
    maka_plugins::computer::Desktop,
);
impl ToolPreparer for Tools {
    fn names(&self) -> Vec<String> {
        definitions().into_iter().map(|d| d.name).collect()
    }
    fn prepare(
        &self,
        name: String,
        input: Value,
        _: ToolCallContext,
        cancellation: CancellationToken,
    ) -> PreparationFuture {
        let computer = self.0.clone();
        let browsers = self.1.clone();
        let desktop = self.2;
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(ToolRejection::Cancelled);
            }
            crate::validate(&name, &input)
                .map_err(|message| ToolRejection::InvalidInput { message })?;
            let title = crate::activity::call(&name, &input);
            Ok(PreparedEffect::new(move |_| {
                Box::pin(async move {
                    let scope = maka_plugins::call::current().ok_or_else(|| {
                        maka_runtime::tools::ToolError::Failed(
                            "Computer Use requires an admitted Agent call".into(),
                        )
                    })?;
                    let result = computer
                        .call(
                            scope,
                            maka_plugins::computer::Call {
                                name,
                                input,
                                browsers,
                                desktop,
                            },
                        )
                        .await?;
                    Ok(ToolOutput::Mcp(result).into())
                })
            })
            .titled(title))
        })
    }
}
