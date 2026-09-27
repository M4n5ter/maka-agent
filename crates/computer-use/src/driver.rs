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

use cua_driver_sdk::{
    ConfiguredDriverOptions, CuaDriver, DriverError, RuntimeAuthorizationOptions,
    SessionPermissionMode, worker::ActionCompletion,
};
use maka_runtime::{capability::CallResult, tools::ToolError};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Active(Arc<CuaDriver>),
    Closed,
}

/// One native runtime per Host process, created only after authorization/T1.
/// Serializing complete calls also keeps observation/action ordering explicit.
#[derive(Default)]
pub struct Driver {
    state: State,
    sessions: std::collections::HashSet<String>,
    pub(crate) cursors: crate::cursor::Host,
}

impl Driver {
    /// Embedding hosts may provide their trusted cursor-capable executable.
    pub fn with_cursor_executable(path: std::path::PathBuf) -> Self {
        Self {
            cursors: crate::cursor::Host::with_executable(path),
            ..Default::default()
        }
    }
    pub async fn release_session(&mut self, session: &str) -> Result<(), ToolError> {
        if self.sessions.contains(session)
            && let State::Active(driver) = &self.state
        {
            driver
                .end_session(cua_driver_contract::EndSessionInput {
                    session: Some(session.into()),
                })
                .await
                .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
            self.sessions.remove(session);
        }
        Ok(())
    }
    pub async fn invoke(
        &mut self,
        name: &str,
        mut input: Value,
        session_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<CallResult, ToolError> {
        if ![
            "list_apps",
            "list_windows",
            "get_window_state",
            "click",
            "type_text",
            "press_key",
            "scroll",
            "drag",
            "set_value",
            "launch_app",
        ]
        .contains(&name)
        {
            return Err(ToolError::Failed("unsupported native operation".into()));
        }
        if cancellation.is_cancelled() {
            return Err(ToolError::Failed(
                "Computer Use cancelled before dispatch".into(),
            ));
        }
        if matches!(self.state, State::Idle) {
            self.state = State::Active(
                CuaDriver::create_configured(ConfiguredDriverOptions {
                    claude_code_compatibility: false,
                    authorization: RuntimeAuthorizationOptions {
                        allowed_modes: vec![SessionPermissionMode::Standard],
                        compatibility_mode: SessionPermissionMode::Standard,
                        compatibility_capability_manifest_path: None,
                        compatibility_bounded_manifest_path: None,
                        unrestricted_acknowledged: false,
                        max_session_ttl_seconds: 86400,
                        max_idle_ttl_seconds: 86400,
                    },
                })
                .map_err(error)?,
            );
        }
        let State::Active(driver) = &self.state else {
            return Err(ToolError::Failed("Computer Use Session is closed".into()));
        };
        if !self.sessions.contains(session_id) {
            driver
                .start_session(cua_driver_contract::StartSessionInput {
                    session: Some(session_id.into()),
                    capture_scope: None,
                    cursor_theme: None,
                })
                .await
                .map_err(error)?;
            self.sessions.insert(session_id.into());
        }
        if cua_driver_contract::tool_input_fields(name)
            .is_some_and(|fields| fields.contains("session"))
            || name == "set_value"
        {
            input
                .as_object_mut()
                .expect("validated object")
                .insert("session".into(), Value::String(session_id.into()));
        }
        // Cancellation prevents queued/new input. Once dispatched, await the
        // native result: dropping an FFI future is not evidence of settlement.
        let result = driver
            .call_tool(name.into(), input.to_string())
            .await
            .map_err(error)?;
        if result.is_error {
            return Err(ToolError::Failed(format!(
                "{}: {}",
                result
                    .error_code
                    .as_deref()
                    .unwrap_or("computer_use_refused"),
                result.text.chars().take(8192).collect::<String>()
            )));
        }
        serde_json::from_str(&result.raw_json)
            .map_err(|error| ToolError::OutcomeUnknown(format!("invalid native result: {error}")))
    }

    /// Host calls this after all Session resources retire.
    pub async fn close(&mut self) -> Result<(), ToolError> {
        self.cursors.stop().await;
        let previous = std::mem::replace(&mut self.state, State::Closed);
        self.sessions.clear();
        if let State::Active(driver) = previous {
            driver
                .shutdown()
                .await
                .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
        }
        Ok(())
    }
}

fn error(error: DriverError) -> ToolError {
    match error {
        DriverError::ActionInterrupted {
            completion: ActionCompletion::NotStarted,
            reason,
        } => ToolError::Failed(reason),
        DriverError::ActionInterrupted { reason, .. } => ToolError::OutcomeUnknown(reason),
        DriverError::Transport { .. }
        | DriverError::Protocol { .. }
        | DriverError::Worker { .. }
        | DriverError::Remote { .. } => ToolError::OutcomeUnknown(error.to_string()),
        error => ToolError::Failed(error.to_string()),
    }
}
