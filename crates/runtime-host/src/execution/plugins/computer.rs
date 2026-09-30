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
use super::{Executions, admission::AgentEvidence};
use maka_plugins::{authorization::Boundary, call::Scope, computer::Call, fiber::Context};
use maka_runtime::capability::CallResult;
use maka_runtime::{
    execution::SandboxMode,
    interaction::{GrantCapability, GrantScope, GrantTarget},
    tool_call::{ToolCallIdentity, ToolOrigin},
    tool_output::ToolOutput,
    tools::{PreparedEffect, ToolCallContext, ToolError, ToolJournal},
};
use maka_tools::ClientInteractions;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

impl Executions {
    pub(crate) async fn plugin_computer_call(
        self: &Arc<Self>,
        owner: Context,
        call: Scope,
        input: Call,
        cancellation: CancellationToken,
    ) -> Result<CallResult, ToolError> {
        let _lease = owner.admit().map_err(failed)?;
        maka_computer_use::validate(&input.name, &input.input).map_err(failed)?;
        let invocation = call
            .identity
            .agent()
            .ok_or_else(|| failed("Computer Use requires an Agent Run"))?
            .clone();
        let mode = computer_mode(
            self.plugin_agent_evidence(&call).await.map_err(failed)?,
            &input.name,
        )?;
        // Human approval is outside the VM's execution deadline. Each native
        // operation still rechecks the live grant under its admission gate.
        if input.name == "cua_repl" && mode == SandboxMode::WorkspaceWrite {
            self.interactions
                .approve(
                    computer_target(),
                    ToolCallContext {
                        invocation: invocation.clone(),
                        operation_id: call
                            .identity
                            .operation_id()
                            .map(str::to_owned)
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    },
                    cancellation.clone(),
                    owner.stopping().map_err(failed)?,
                )
                .await
                .map_err(failed)?;
        }
        let interaction = self.computer.get(&invocation.session_id)?;
        let mut repl = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(failed("Computer Use cancelled while queued")),
            repl = interaction.repl.lock() => repl,
        };
        let admission = self.interactions.own_admission().await;
        let current_mode = computer_mode(
            self.plugin_agent_evidence(&call).await.map_err(failed)?,
            &input.name,
        )?;
        let current = self
            .log
            .get_session::<crate::session::SessionConfiguration>(&invocation.session_id)
            .await
            .map_err(|error| ToolError::Persistence(error.to_string()))?
            .ok_or_else(|| failed("Computer Use Session is unavailable"))?;
        if current
            .configuration
            .bound_tools
            .as_ref()
            .is_some_and(|tools| !tools.contains(&input.name))
        {
            return Err(failed(
                "Computer Use tool is outside the current Session ceiling",
            ));
        }
        if cancellation.is_cancelled() || !owner.is_effective() {
            return Err(failed("Computer Use authority retired before evaluation"));
        }
        if input.name == "cua_repl"
            && current_mode == SandboxMode::WorkspaceWrite
            && self
                .log
                .client_capability_grant(&invocation.session_id, &computer_target())
                .await
                .map_err(|error| ToolError::Persistence(error.to_string()))?
                .is_none()
        {
            return Err(failed("Computer Use requires a current Session grant"));
        }
        drop(admission);
        if input.name == "cua_reset" {
            if let Some(runtime) = repl.take() {
                runtime
                    .close()
                    .await
                    .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
            }
            let mut native = interaction.native.lock().await;
            native.close().await?;
            *native = maka_computer_use::Session::new(self.computer.driver.clone());
            return Ok(CallResult {
                content: vec![maka_runtime::capability::ContentBlock::Text {
                    text: "Computer Use REPL reset. Bind targets again.".into(),
                }],
                structured_content: None,
            });
        }
        let platform = {
            let mut native = interaction.native.lock().await;
            native.configure(input.desktop, input.browsers)?;
            native.prepare(&cancellation).await?;
            native.platform()
        };
        let input: maka_computer_use::Evaluate =
            serde_json::from_value(input.input).map_err(failed)?;
        let limits = maka_js_runtime::CellLimits {
            max_value_bytes: 16 * 1024 * 1024,
            heap_bytes: 128 * 1024 * 1024,
            ..Default::default()
        };
        let context =
            maka_js_runtime::CellContext::new(Default::default(), limits.max_value_bytes, vec![])
                .with_image_deduplication();
        let runtime = repl.get_or_insert_with(|| {
            maka_js_runtime::repl::Repl::new(maka_computer_use::facade_for(platform), limits)
        });
        let bridge = Arc::new(Bridge {
            host: self.clone(),
            owner,
            call,
            native: interaction.native.clone(),
        });
        let result = runtime
            .evaluate(
                input.code,
                bridge,
                cancellation,
                context.clone(),
                std::time::Duration::from_millis(input.timeout_ms),
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(maka_js_runtime::CellAbort::Tool(error)) => return Err(error),
            Err(error) => return Err(failed(error)),
        };
        if let maka_js_runtime::CellResult::Failure { error, .. } = result {
            return Err(failed(format!("Cua JavaScript error: {}", error.message)));
        }
        let mut content = Vec::new();
        for output in context.take_output() {
            match output {
                maka_js_runtime::CellOutput::Text { text } => {
                    content.push(maka_runtime::capability::ContentBlock::Text { text })
                }
                maka_js_runtime::CellOutput::Media { content: block, .. } => content.push(block),
                maka_js_runtime::CellOutput::Image { .. } => {
                    return Err(failed("unexpected stored image in Cua REPL"));
                }
            }
        }
        Ok(CallResult {
            content,
            structured_content: Some(serde_json::json!({"ok":true})),
        })
    }

    async fn plugin_computer_operation(
        self: &Arc<Self>,
        owner: Context,
        call: Scope,
        input: Value,
        native: Arc<tokio::sync::Mutex<maka_computer_use::Session>>,
        cancellation: CancellationToken,
    ) -> Result<Value, ToolError> {
        let _lease = owner.admit().map_err(failed)?;
        let identity = owner.identity().map_err(failed)?;
        let command: maka_computer_use::protocol::Command =
            serde_json::from_value(input.clone()).map_err(failed)?;
        let invocation = call
            .identity
            .agent()
            .ok_or_else(|| failed("Computer Use requires an Agent Run"))?
            .clone();
        let evidence = self.plugin_agent_evidence(&call).await.map_err(failed)?;
        computer_mode(evidence, "cua_repl")?;
        let context = ToolCallContext {
            invocation: invocation.clone(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        };
        let target = computer_target();
        // Queue before the final authority check. A mode change while another
        // native call runs cannot leave this queued call with stale permission.
        let device = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(failed("Computer Use cancelled while queued")),
            device = self.computer.input.clone().lock_owned() => device,
        };
        let mut runtime = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(failed("Computer Use cancelled while queued")),
            runtime = native.lock_owned() => runtime,
        };
        let gate = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(failed("Computer Use cancelled before admission")),
            gate = self.interactions.own_admission() => gate,
        };
        let current = self.plugin_agent_evidence(&call).await.map_err(failed)?;
        let current_mode = computer_mode(current, "cua_repl")?;
        let session = self
            .log
            .get_session::<crate::session::SessionConfiguration>(&invocation.session_id)
            .await
            .map_err(|error| ToolError::Persistence(error.to_string()))?
            .ok_or_else(|| failed("Computer Use Session is unavailable"))?;
        if session
            .configuration
            .bound_tools
            .as_ref()
            .is_some_and(|tools| !tools.contains("cua_repl"))
        {
            return Err(failed(
                "Computer Use tool is outside the current Session ceiling",
            ));
        }
        if current_mode == SandboxMode::WorkspaceWrite
            && self
                .log
                .client_capability_grant(&invocation.session_id, &target)
                .await
                .map_err(|e| ToolError::Persistence(e.to_string()))?
                .is_none()
        {
            return Err(failed("Computer Use requires a current Session grant"));
        }
        if cancellation.is_cancelled() || !owner.is_effective() {
            return Err(failed("Computer Use authority retired before admission"));
        }
        let title = runtime.activity(&command);
        let session_id = invocation.session_id.clone();
        let effect = PreparedEffect::new(move |cancellation| {
            Box::pin(async move {
                // T1 committed under the shared admission gate. Native work never
                // holds that gate, so Stop and unrelated Sessions remain responsive.
                drop(gate);
                let _device = device;
                let mut output = runtime.invoke(command, &session_id, &cancellation).await?;
                let image = output
                    .as_object_mut()
                    .and_then(|object| object.remove("image"));
                let content = image
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(failed)?
                    .into_iter()
                    .collect();
                Ok(ToolOutput::Mcp(CallResult {
                    content,
                    structured_content: Some(output),
                })
                .into())
            })
        });
        let effect = effect.titled(title);
        let result = ToolJournal::new(self.log.clone(), invocation)
            .invoke_prepared_output(
                context.operation_id,
                ToolCallIdentity {
                    tool_call_id: uuid::Uuid::new_v4().to_string(),
                    origin: ToolOrigin::HostSdk {
                        package_id: identity.package_id,
                        entry_id: identity.entry_id,
                        activation: identity.activation,
                        parent_operation_id: call.identity.operation_id().map(str::to_owned),
                    },
                },
                "cua_operation".into(),
                input,
                cancellation,
                effect,
            )
            .await;
        if matches!(
            result,
            Err(ToolError::Persistence(_) | ToolError::CleanupUnconfirmed(_))
        ) {
            self.begin_drain();
        }
        match result? {
            ToolOutput::Mcp(result) => {
                let mut value = result.structured_content.unwrap_or(Value::Null);
                if let Some(image) = result.content.into_iter().find(|block| {
                    matches!(block, maka_runtime::capability::ContentBlock::Image { .. })
                }) {
                    value["image"] = serde_json::to_value(image).map_err(failed)?;
                }
                Ok(value)
            }
            _ => Err(ToolError::CleanupUnconfirmed(
                "Computer Use result changed format".into(),
            )),
        }
    }
}

struct Bridge {
    host: Arc<Executions>,
    owner: Context,
    call: Scope,
    native: Arc<tokio::sync::Mutex<maka_computer_use::Session>>,
}
impl maka_runtime::tools::ToolExecutor for Bridge {
    fn names(&self) -> Vec<String> {
        vec!["cua".into()]
    }
    fn invoke(
        &self,
        _: String,
        input: Value,
        cancellation: CancellationToken,
    ) -> maka_runtime::tools::ToolFuture {
        let host = self.host.clone();
        let owner = self.owner.clone();
        let call = self.call.clone();
        let native = self.native.clone();
        Box::pin(async move {
            host.plugin_computer_operation(owner, call, input, native, cancellation)
                .await
        })
    }
}

fn computer_target() -> GrantTarget {
    GrantTarget {
        provider_id: "maka_computer_use_host".into(),
        contract_id: maka_computer_use::REVISION.into(),
        server_id: "maka_computer_use".into(),
        tool_name: "cua_repl".into(),
        capability: GrantCapability::ComputerUse,
        scope: GrantScope::Capability {},
    }
}

fn computer_mode(evidence: &AgentEvidence, name: &str) -> Result<SandboxMode, ToolError> {
    let Boundary::Session { boundary, .. } = &evidence.boundary else {
        return Err(failed("Computer Use requires a Session"));
    };
    if boundary.sandbox_mode == SandboxMode::ReadOnly {
        return Err(failed("Explore mode does not allow Computer Use"));
    }
    if evidence
        .invocation
        .tool_composition
        .as_ref()
        .and_then(|composition| composition.bound_tools.as_ref())
        .is_some_and(|tools| !tools.contains(name))
    {
        return Err(failed("Computer Use tool is outside the Run tool ceiling"));
    }
    Ok(boundary.sandbox_mode)
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
