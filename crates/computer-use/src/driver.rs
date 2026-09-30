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
    ConfiguredDriverOptions, CuaDriver, CuaDriverSession, DriverError, RuntimeAuthorizationOptions,
    SessionPermissionMode, TrustedSessionOptions, worker::ActionCompletion,
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
    sessions: std::collections::HashMap<String, Arc<CuaDriverSession>>,
    pub(crate) cursors: crate::cursor::Host,
    pub(crate) windows: crate::desktop::worker::Worker,
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
        if let Some(bound) = self.sessions.remove(session) {
            let ended = bound
                .end_session(cua_driver_contract::EndSessionInput { session: None })
                .await;
            // Always revoke the owned lease, including after expiry. A refused
            // end cannot dispatch native work; its old private episode is left
            // to the SDK's bounded idle cleanup. Transport/cleanup uncertainty
            // still stops the Host rather than fabricating settlement.
            bound.close();
            match ended {
                Ok(_) | Err(DriverError::Shutdown) => {}
                Err(DriverError::Tool { error_code, .. })
                    if matches!(
                        error_code.as_str(),
                        "permission_denied" | "session_ended" | "authorization_revoked"
                    ) => {}
                Err(error) => return Err(ToolError::CleanupUnconfirmed(error.to_string())),
            }
        }
        Ok(())
    }
    pub async fn invoke(
        &mut self,
        name: &str,
        input: Value,
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
        let session = self.session(session_id).await?;
        // Every operation, including inventory whose schema has no `session`
        // field, uses the same bound SDK surface. Once dispatched, await its
        // result: dropping an FFI future is not evidence of settlement.
        let result = session
            .call_tool(name.into(), input.to_string())
            .await
            .map_err(error)?;
        if result.is_error {
            let code = result
                .error_code
                .as_deref()
                .unwrap_or("computer_use_refused");
            let message = refusal(code, &result.text);
            return Err(ToolError::Failed(format!("{code}: {message}")));
        }
        serde_json::from_str(&result.raw_json)
            .map_err(|error| ToolError::OutcomeUnknown(format!("invalid native result: {error}")))
    }

    async fn session(&mut self, id: &str) -> Result<&CuaDriverSession, ToolError> {
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
                // No operation has been dispatched during construction. In
                // particular, a non-interactive desktop is a known refusal.
                .map_err(|error| ToolError::Failed(error.to_string()))?,
            );
        }
        let State::Active(driver) = &self.state else {
            return Err(ToolError::Failed("Computer Use Session is closed".into()));
        };
        if !self.sessions.contains_key(id) {
            let session = driver
                .create_trusted_session(TrustedSessionOptions {
                    // SDK lifecycle tombstones belong to their original transport.
                    // A reset owns a fresh episode, never revives an old lease's ID.
                    public_session: format!("maka-{}", uuid::Uuid::new_v4()),
                    mode: SessionPermissionMode::Standard,
                    ttl_seconds: 86400,
                    idle_ttl_seconds: 86400,
                    capability_manifest_path: None,
                    bounded_manifest_path: None,
                })
                .map_err(error)?;
            session
                .start_session(cua_driver_contract::StartSessionInput {
                    session: None,
                    capture_scope: None,
                    cursor_theme: None,
                })
                .await
                .map_err(error)?;
            self.sessions.insert(id.into(), session);
        }
        Ok(self.sessions.get(id).expect("bound session created"))
    }

    /// Host calls this after all Session resources retire.
    pub async fn close(&mut self) -> Result<(), ToolError> {
        let failure = self.windows.close().await.err();
        self.cursors.stop().await;
        let previous = std::mem::replace(&mut self.state, State::Closed);
        self.sessions.clear();
        if let State::Active(driver) = previous {
            driver
                .shutdown()
                .await
                .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
        }
        failure.map_or(Ok(()), Err)
    }
}

fn refusal(code: &str, reason: &str) -> String {
    let terminal = matches!(code, "session_ended" | "authorization_revoked")
        // The pinned SDK reports expiry under permission_denied, without a
        // structured subcode. Keep this adaptation at its native boundary;
        // ordinary permission refusals must not imply that reset grants access.
        || (code == "permission_denied" && reason.contains("authorization context expired"));
    let reason = if code == "session_ended" {
        "this session has ended"
    } else {
        reason
    };
    let mut message: String = reason.chars().take(8192).collect();
    if terminal {
        message.push_str("\nCall cua_reset, then bind and observe the current app or tab before acting. This refused operation did not dispatch input.");
    }
    message
}

fn error(error: DriverError) -> ToolError {
    match error {
        DriverError::Shutdown => ToolError::Failed(
            "Computer Use session is no longer active. Call cua_reset, then bind and observe the current app or tab before acting.".into(),
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use cua_driver_contract::{EndSessionInput, GetSessionInput, StartSessionInput};

    // This uses only SDK session administration, never desktop observation or
    // input. It exercises the real binding/expiry/reset ownership contract.
    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "the SDK requires an interactive Windows desktop even for lifecycle administration"
    )]
    async fn bound_sessions_ignore_ended_implicit_state_and_reset_independently() {
        let mut driver = Driver::default();
        driver.session("a").await.unwrap();
        driver.session("b").await.unwrap();
        let a = driver.sessions["a"].clone();
        let b = driver.sessions["b"].clone();
        let State::Active(native) = &driver.state else {
            unreachable!()
        };
        native
            .start_session(StartSessionInput {
                session: None,
                capture_scope: None,
                cursor_theme: None,
            })
            .await
            .unwrap();
        native
            .end_session(EndSessionInput { session: None })
            .await
            .unwrap();
        let before = a
            .get_session(GetSessionInput { session: None })
            .await
            .unwrap();
        let other = b
            .get_session(GetSessionInput { session: None })
            .await
            .unwrap();
        assert_ne!(before.session, other.session);
        assert!(
            before.expires_in_seconds > 300,
            "bound lifecycle must use its explicit idle lease"
        );
        driver.release_session("a").await.unwrap();
        assert!(
            a.get_session(GetSessionInput { session: None })
                .await
                .is_err()
        );
        let after = driver
            .session("a")
            .await
            .unwrap()
            .get_session(GetSessionInput { session: None })
            .await
            .unwrap();
        assert_ne!(
            before.session, after.session,
            "reset owns a fresh lifecycle episode"
        );
        assert_eq!(
            other.session,
            b.get_session(GetSessionInput { session: None })
                .await
                .unwrap()
                .session
        );
        // Terminal authority must not make reset itself impossible.
        b.end_session(EndSessionInput { session: None })
            .await
            .unwrap();
        driver.release_session("b").await.unwrap();
        driver
            .session("b")
            .await
            .unwrap()
            .get_session(GetSessionInput { session: None })
            .await
            .unwrap();
        driver.release_session("a").await.unwrap();
        driver.release_session("b").await.unwrap();
        // Exercise the SDK's real expiry diagnostic, not a fabricated error.
        let State::Active(native) = &driver.state else {
            unreachable!()
        };
        let expired = native
            .create_trusted_session(TrustedSessionOptions {
                public_session: format!("expiry-{}", uuid::Uuid::new_v4()),
                mode: SessionPermissionMode::Standard,
                ttl_seconds: 1,
                idle_ttl_seconds: 1,
                capability_manifest_path: None,
                bounded_manifest_path: None,
            })
            .unwrap();
        expired
            .start_session(StartSessionInput {
                session: None,
                capture_scope: None,
                cursor_theme: None,
            })
            .await
            .unwrap();
        driver.sessions.insert("expired".into(), expired);
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        // SDK authorization rejects this before any desktop observation.
        let failure = driver
            .invoke(
                "list_apps",
                serde_json::json!({}),
                "expired",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            failure.contains("authorization context expired"),
            "{failure}"
        );
        assert!(failure.contains("cua_reset"), "{failure}");
        assert!(
            !refusal(
                "permission_denied",
                "desktop observation is forbidden by policy"
            )
            .contains("cua_reset")
        );
        driver.release_session("expired").await.unwrap();
        driver
            .session("expired")
            .await
            .unwrap()
            .get_session(GetSessionInput { session: None })
            .await
            .unwrap();
        driver.release_session("expired").await.unwrap();
        driver.close().await.unwrap();
    }
}
