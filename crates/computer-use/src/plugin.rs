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
    fn description(&self) -> Option<maka_plugins::kernel::Description> {
        Some(maka_plugins::kernel::Description {
            name: maka_plugins::terminal_ui::Text::localized(
                "Computer use",
                "电脑操作",
                "電腦操作",
            ),
            summary: Some(maka_plugins::terminal_ui::Text::localized(
                "Interact with apps and browser tabs.",
                "操作应用和浏览器标签页。",
                "操作應用程式與瀏覽器分頁。",
            )),
        })
    }

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
            staged
                .insert(
                    "computer-state",
                    maka_plugins::prompt::DynamicContext {
                        format: maka_plugins::prompt::Format::Plain,
                        order: 0,
                        text: maka_plugins::prompt::Text::Dynamic(handler.clone()),
                    },
                )
                .map_err(|error| error.to_string())?;
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

impl maka_plugins::prompt::Provider for Tools {
    fn evaluate(
        &self,
        request: maka_plugins::prompt::Request,
        _: maka_plugins::filesystem::ReadDirectory,
    ) -> maka_plugins::prompt::TextFuture {
        if !request
            .tools
            .iter()
            .any(|name| matches!(name.as_str(), "cua_repl" | "cua_reset"))
        {
            return Box::pin(async { Ok(None) });
        }
        let computer = self.0.clone();
        let browsers = self
            .1
            .iter()
            .map(|browser| browser.id.clone())
            .collect::<Vec<_>>();
        Box::pin(async move {
            use maka_plugins::computer::ReplState;
            let state = tokio::select! {
                biased;
                _ = request.cancellation.cancelled() => return Err(maka_plugins::Error::Retired),
                state = computer.state(request.target.session_id().into()) => state?,
            };
            let mut text = String::from(
                "Computer Use state captured before this model step's first attempt. Later CUA tool results supersede this snapshot. ",
            );
            text.push_str(match state {
                ReplState::Fresh => "The REPL was fresh; earlier runtime variables and app/tab bindings did not exist. If no later call has initialized it, start with exactly one entry point: await cua.getState(), const app = await cua.getApp(...), or const tab = await cua.getTab(...), then read its documentation and initial state.",
                ReplState::Ready => "The REPL was running with its existing variables and bindings.",
                ReplState::ResetRequired => "The REPL was stopped. Call cua_reset unless a later reset already succeeded, then select and observe the current app/tab. An interrupted input may have taken effect; inspect before repeating it.",
            });
            text.push_str(" Bindings created afterward can be reused unless a later reset or terminated REPL invalidated them. Inspect the current UI before acting.");
            if browsers.is_empty() {
                text.push_str(" No browser-tab provider is configured. Use cua.getApp for native browser windows; the Chrome extension/IAB IDs from other agents are not available here.");
            } else {
                text.push_str(&format!(" Configured browser providers: {}. Select them through cua.getState/getBrowser; availability is checked on discovery.", serde_json::to_string(&browsers).expect("browser IDs serialize")));
            }
            Ok(Some(text))
        })
    }
}
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
