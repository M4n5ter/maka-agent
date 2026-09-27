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

//! Select the desktop independently of the OS running the Agent.
use crate::{Driver, protocol::Command, session};
use maka_plugins::computer::{BrowserConnection, Desktop};
use maka_runtime::tools::ToolError;
use serde_json::Value;
use std::sync::Arc;
use std::{path::PathBuf, sync::OnceLock};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub(crate) mod worker;
pub use worker::bootstrap;

/// The embedding application owns distribution and version selection. Neither
/// the model nor plugin configuration can provide a downloader or executable.
pub type WindowsHelperResolver = fn() -> maka_runtime::tools::ToolFuture<PathBuf>;
static WINDOWS_HELPER: OnceLock<WindowsHelperResolver> = OnceLock::new();

pub fn register_windows_helper(resolver: WindowsHelperResolver) {
    assert!(
        WINDOWS_HELPER.set(resolver).is_ok(),
        "Windows helper resolver is configured once at application startup"
    );
}

pub(crate) fn local_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "mac",
        other => other,
    }
}

fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

fn use_windows(desktop: Desktop, platform: &str, wsl: bool) -> Result<bool, ToolError> {
    match desktop {
        Desktop::Local => Ok(false),
        Desktop::Auto => Ok(platform == "linux" && wsl),
        Desktop::Windows if platform == "windows" => Ok(false),
        Desktop::Windows if platform == "linux" && wsl => Ok(true),
        Desktop::Windows => Err(failed("Windows desktop requires Windows or WSL")),
    }
}

/// The selected desktop owns all targets, including browser connections. The
/// embedding Host still admits and journals every operation before invoking it.
pub struct Session {
    driver: Arc<Mutex<Driver>>,
    backend: Option<Backend>,
    closed: bool,
}
enum Backend {
    Local(session::Session),
    Windows {
        id: Option<String>,
        browsers: Vec<BrowserConnection>,
    },
}
impl Session {
    pub fn new(driver: Arc<Mutex<Driver>>) -> Self {
        Self {
            driver,
            backend: None,
            closed: false,
        }
    }

    pub fn configure(
        &mut self,
        desktop: Desktop,
        browsers: Vec<BrowserConnection>,
    ) -> Result<(), ToolError> {
        if self.closed {
            return Err(failed("Computer Use Session is closed"));
        }
        crate::browser::validate_connections(&browsers)?;
        let windows = use_windows(desktop, local_platform(), is_wsl())?;
        if let Some(backend) = &self.backend
            && windows != matches!(backend, Backend::Windows { .. })
        {
            return Err(failed(
                "desktop configuration changed; reset Cua before switching desktops",
            ));
        }
        let backend = self.backend.get_or_insert_with(|| {
            if windows {
                Backend::Windows {
                    id: None,
                    browsers: Vec::new(),
                }
            } else {
                Backend::Local(session::Session::new(self.driver.clone()))
            }
        });
        match backend {
            Backend::Local(local) => local.configure_browsers(browsers),
            Backend::Windows {
                browsers: current, ..
            } => {
                *current = browsers;
                Ok(())
            }
        }
    }

    pub fn configure_browsers(
        &mut self,
        browsers: Vec<BrowserConnection>,
    ) -> Result<(), ToolError> {
        self.configure(Desktop::Auto, browsers)
    }

    pub fn platform(&self) -> &'static str {
        if matches!(self.backend, Some(Backend::Windows { .. })) {
            "windows"
        } else {
            local_platform()
        }
    }

    /// Prepare dependencies and the private channel outside the REPL deadline.
    /// This does not create a native Cua runtime or dispatch any input.
    pub async fn prepare(&mut self, cancellation: &CancellationToken) -> Result<(), ToolError> {
        if cancellation.is_cancelled() {
            return Err(failed("Computer Use preparation cancelled"));
        }
        if self.closed {
            return Err(failed("Computer Use Session is closed"));
        }
        if self.backend.is_none() {
            self.configure(Desktop::Auto, Vec::new())?;
        }
        if matches!(self.backend, Some(Backend::Windows { .. })) {
            let mut driver = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(failed("Computer Use preparation cancelled")),
                driver = self.driver.lock() => driver,
            };
            driver.windows.prepare(cancellation).await?;
        }
        Ok(())
    }

    pub async fn invoke(
        &mut self,
        command: Command,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<Value, ToolError> {
        if cancellation.is_cancelled() {
            return Err(failed("Computer Use cancelled before dispatch"));
        }
        if self.closed {
            return Err(failed("Computer Use Session is closed"));
        }
        if self.backend.is_none() {
            self.configure(Desktop::Auto, Vec::new())?;
        }
        match self.backend.as_mut().unwrap() {
            Backend::Local(local) => local.invoke(command, session, cancellation).await,
            Backend::Windows { id, browsers } => {
                if id.as_deref().is_some_and(|id| id != session) {
                    return Err(failed("Computer Use Session identity mismatch"));
                }
                id.get_or_insert_with(|| session.into());
                let mut driver = self.driver.lock().await;
                if cancellation.is_cancelled() {
                    return Err(failed("Computer Use cancelled before dispatch"));
                }
                driver
                    .windows
                    .call(
                        worker::Request::Invoke {
                            session: session.into(),
                            browsers: browsers.clone(),
                            command,
                        },
                        cancellation,
                    )
                    .await
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), ToolError> {
        self.closed = true;
        match &mut self.backend {
            Some(Backend::Local(local)) => local.close().await,
            Some(Backend::Windows { id: Some(id), .. }) => {
                self.driver.lock().await.windows.release_session(id).await?;
                self.backend = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_selection_never_falls_back_from_windows() {
        for platform in ["linux", "mac", "windows"] {
            for wsl in [false, true] {
                assert!(!use_windows(Desktop::Local, platform, wsl).unwrap());
                assert_eq!(
                    use_windows(Desktop::Auto, platform, wsl).unwrap(),
                    platform == "linux" && wsl
                );
                let result = use_windows(Desktop::Windows, platform, wsl);
                match platform {
                    "windows" => assert!(!result.unwrap()),
                    "linux" if wsl => assert!(result.unwrap()),
                    _ => assert!(
                        matches!(result, Err(ToolError::Failed(message)) if message == "Windows desktop requires Windows or WSL")
                    ),
                }
            }
        }
    }
}
