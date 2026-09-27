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

use maka_runtime::tools::ToolError;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    process::{ExitCode, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

mod helper;
pub mod theme;

const ENTRY: &str = "__maka-cua-cursor";
const MAX_MESSAGE: u64 = 16 * 1024;
static HOST_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();
static NEXT_COLOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Call at the executable entry point, before starting runtime threads.
/// The private child only renders cursors; it has no computer input operations.
pub fn bootstrap() -> Option<ExitCode> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(ENTRY)) {
        return Some(match helper::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                use std::io::Write as _;
                let _ = writeln!(
                    std::io::stdout(),
                    "{}",
                    serde_json::to_string(&RenderState::unavailable(&error)).unwrap()
                );
                let _ = writeln!(std::io::stderr(), "Maka cursor: {error}");
                ExitCode::FAILURE
            }
        });
    }
    if let Ok(path) = std::env::current_exe() {
        let _ = HOST_EXECUTABLE.set(path);
    }
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Blue,
    Mint,
    Violet,
    Amber,
    Rose,
    Cyan,
}
impl Color {
    pub const ALL: [Self; 6] = [
        Self::Blue,
        Self::Mint,
        Self::Violet,
        Self::Amber,
        Self::Rose,
        Self::Cyan,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Mint => "mint",
            Self::Violet => "violet",
            Self::Amber => "amber",
            Self::Rose => "rose",
            Self::Cyan => "cyan",
        }
    }
    pub fn rgba(self) -> [u8; 4] {
        match self {
            Self::Blue => [102, 158, 255, 255],
            Self::Mint => [41, 176, 147, 255],
            Self::Violet => [155, 123, 237, 255],
            Self::Amber => [224, 159, 48, 255],
            Self::Rose => [228, 112, 153, 255],
            Self::Cyan => [48, 170, 211, 255],
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub label: Option<String>,
    pub color: Option<Color>,
    pub enabled: Option<bool>,
    pub reduced_motion: Option<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RenderStatus {
    #[default]
    Idle,
    Ready,
    Unavailable,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenderState {
    pub status: RenderStatus,
    pub visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_position: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renderer_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl RenderState {
    pub(crate) fn unavailable(error: impl std::fmt::Display) -> Self {
        Self {
            status: RenderStatus::Unavailable,
            error: Some(error.to_string()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Spec {
    pub id: String,
    pub label: String,
    pub color: Color,
    pub enabled: bool,
    pub reduced_motion: bool,
}
impl Default for Spec {
    fn default() -> Self {
        let id = uuid::Uuid::new_v4();
        Self {
            color: Color::ALL
                [NEXT_COLOR.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % Color::ALL.len()],
            id: id.to_string(),
            label: "Maka".into(),
            enabled: true,
            reduced_motion: false,
        }
    }
}
impl Spec {
    pub(crate) fn configure(&mut self, options: Options) -> Result<(), ToolError> {
        if let Some(label) = &options.label
            && (label.chars().count() > 48 || label.chars().any(char::is_control))
        {
            return Err(failed(
                "cursor label must be at most 48 characters without control characters",
            ));
        }
        if let Some(label) = options.label {
            self.label = label;
        }
        if let Some(color) = options.color {
            self.color = color;
        }
        if let Some(enabled) = options.enabled {
            self.enabled = enabled;
        }
        if let Some(reduced) = options.reduced_motion {
            self.reduced_motion = reduced;
        }
        Ok(())
    }
    pub(crate) fn theme_id(&self) -> String {
        format!("org.apache.maka.cursor.{}", self.color.name())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Update {
        cursor: Spec,
        point: Option<[f64; 2]>,
        action: cursor_overlay::CursorAction,
        window: Option<u64>,
    },
    State {
        id: String,
    },
    Remove {
        id: String,
    },
}

pub(crate) struct Host {
    executable: Option<PathBuf>,
    sender: Option<tokio::sync::mpsc::Sender<Job>>,
}
struct Job {
    command: Control,
    reply: Option<tokio::sync::oneshot::Sender<Result<RenderState, ToolError>>>,
    queued: std::time::Instant,
}
enum Control {
    Call(Request),
    Configure(Spec),
    Stop,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            executable: HOST_EXECUTABLE.get().cloned(),
            sender: None,
        }
    }
}
impl Host {
    pub(crate) fn with_executable(path: PathBuf) -> Self {
        Self {
            executable: Some(path),
            sender: None,
        }
    }
    pub(crate) fn registered(&self) -> bool {
        self.executable.is_some()
    }
    fn sender(&mut self) -> tokio::sync::mpsc::Sender<Job> {
        self.sender
            .get_or_insert_with(|| {
                let (sender, mut receiver) = tokio::sync::mpsc::channel::<Job>(32);
                let mut transport = Transport {
                    executable: self.executable.clone(),
                    process: None,
                    last_error: None,
                };
                tokio::spawn(async move {
                    while let Some(job) = receiver.recv().await {
                        if job.reply.is_none() && job.queued.elapsed() > Duration::from_secs(1) {
                            continue;
                        }
                        let stopping = matches!(job.command, Control::Stop);
                        let result = match job.command {
                            Control::Call(Request::State { id }) => Ok(transport.state(&id).await),
                            Control::Call(Request::Remove { id }) => {
                                transport.remove(&id).await;
                                Ok(RenderState::default())
                            }
                            Control::Call(request) => transport.request(request).await,
                            Control::Configure(cursor) => Ok(transport.configure(&cursor).await),
                            Control::Stop => {
                                transport.stop().await;
                                Ok(RenderState::default())
                            }
                        };
                        if let Some(reply) = job.reply {
                            let _ = reply.send(result);
                        }
                        if stopping {
                            break;
                        }
                    }
                    transport.stop().await;
                });
                sender
            })
            .clone()
    }
    async fn call(&mut self, command: Control) -> Result<RenderState, ToolError> {
        if self
            .sender
            .as_ref()
            .is_some_and(tokio::sync::mpsc::Sender::is_closed)
        {
            self.sender = None;
        }
        let (reply, receive) = tokio::sync::oneshot::channel();
        self.sender()
            .send(Job {
                command,
                reply: Some(reply),
                queued: std::time::Instant::now(),
            })
            .await
            .map_err(|_| failed("cursor renderer owner closed"))?;
        receive
            .await
            .map_err(|_| failed("cursor renderer owner stopped"))?
    }
    pub(crate) fn cue(
        &mut self,
        cursor: &Spec,
        point: [f64; 2],
        action: cursor_overlay::CursorAction,
        window: u64,
    ) {
        if !self.registered() {
            return;
        }
        let _ = self.sender().try_send(Job {
            command: Control::Call(Request::Update {
                cursor: cursor.clone(),
                point: Some(point),
                action,
                window: Some(window),
            }),
            reply: None,
            queued: std::time::Instant::now(),
        });
    }
    pub(crate) async fn update(
        &mut self,
        cursor: &Spec,
        point: Option<[f64; 2]>,
        action: cursor_overlay::CursorAction,
        window: Option<u64>,
    ) -> Result<RenderState, ToolError> {
        self.call(Control::Call(Request::Update {
            cursor: cursor.clone(),
            point,
            action,
            window,
        }))
        .await
    }
    pub(crate) async fn state(&mut self, cursor: &Spec) -> RenderState {
        self.call(Control::Call(Request::State {
            id: cursor.id.clone(),
        }))
        .await
        .unwrap_or_else(RenderState::unavailable)
    }
    pub(crate) async fn configure(&mut self, cursor: &Spec) -> RenderState {
        self.call(Control::Configure(cursor.clone()))
            .await
            .unwrap_or_else(RenderState::unavailable)
    }
    pub(crate) async fn remove(&mut self, id: &str) {
        if self.sender.is_some() {
            let _ = self
                .call(Control::Call(Request::Remove { id: id.into() }))
                .await;
        }
    }
    pub(crate) async fn stop(&mut self) {
        if self.sender.is_some() {
            let _ = self.call(Control::Stop).await;
            self.sender = None;
        }
    }
}

struct Transport {
    executable: Option<PathBuf>,
    process: Option<Process>,
    last_error: Option<String>,
}
struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    _themes: tempfile::TempDir,
}
impl Transport {
    async fn start(&mut self) -> Result<(), ToolError> {
        if self.process.is_some() {
            return Ok(());
        }
        let executable=self.executable.as_ref().ok_or_else(|| failed("cursor_unavailable: this executable did not register the native cursor entry point"))?;
        let themes = tempfile::Builder::new()
            .prefix("maka-cursors-")
            .tempdir()
            .map_err(failed)?;
        theme::write_artifacts(themes.path()).map_err(failed)?;
        let mut command = tokio::process::Command::new(executable);
        command
            .arg(ENTRY)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .env_clear();
        for name in [
            "HOME",
            "PATH",
            "LANG",
            "LC_ALL",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "LOCALAPPDATA",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("CUA_DRIVER_CURSOR_THEME_DIR", themes.path());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        #[cfg(windows)]
        command.creation_flags(
            windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
                | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
        );
        let mut child = command.spawn().map_err(failed)?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| failed("cursor stdin unavailable"))?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| failed("cursor stdout unavailable"))?,
        );
        self.process = Some(Process {
            child,
            input,
            output,
            _themes: themes,
        });
        Ok(())
    }
    async fn request(&mut self, request: Request) -> Result<RenderState, ToolError> {
        if let Some(error) = &self.last_error {
            return Err(failed(error));
        }
        let starting = self.process.is_none();
        if let Err(error) = self.start().await {
            self.last_error = Some(error.to_string());
            return Err(error);
        }
        let process = self.process.as_mut().unwrap();
        let result =
            tokio::time::timeout(Duration::from_secs(if starting { 10 } else { 3 }), async {
                let mut bytes = serde_json::to_vec(&request).map_err(failed)?;
                bytes.push(b'\n');
                process.input.write_all(&bytes).await.map_err(failed)?;
                let mut line = String::new();
                (&mut process.output)
                    .take(MAX_MESSAGE + 1)
                    .read_line(&mut line)
                    .await
                    .map_err(failed)?;
                if line.is_empty() || line.len() as u64 > MAX_MESSAGE {
                    return Err(failed("cursor renderer returned an invalid response"));
                }
                let value: RenderState = serde_json::from_str(&line).map_err(failed)?;
                if let Some(error) = &value.error {
                    return Err(failed(error));
                }
                Ok(value)
            })
            .await
            .unwrap_or_else(|_| Err(failed("cursor renderer did not respond")));
        if let Err(error) = &result {
            self.last_error = Some(error.to_string());
            self.stop().await;
        }
        result
    }
    pub(crate) async fn update(
        &mut self,
        cursor: &Spec,
        point: Option<[f64; 2]>,
        action: cursor_overlay::CursorAction,
        window: Option<u64>,
    ) -> Result<RenderState, ToolError> {
        self.request(Request::Update {
            cursor: cursor.clone(),
            point,
            action,
            window,
        })
        .await
    }
    async fn state(&mut self, id: &str) -> RenderState {
        if self.process.is_none() {
            return match &self.last_error {
                Some(error) => RenderState::unavailable(error),
                None if self.executable.is_none() => {
                    RenderState::unavailable("no cursor display host registered")
                }
                None => RenderState::default(),
            };
        }
        match self.request(Request::State { id: id.into() }).await {
            Ok(value) => value,
            Err(error) => RenderState::unavailable(error),
        }
    }
    pub(crate) async fn configure(&mut self, cursor: &Spec) -> RenderState {
        self.last_error = None;
        if self.process.is_some() {
            match self
                .update(cursor, None, cursor_overlay::CursorAction::Idle, None)
                .await
            {
                Ok(value) => value,
                Err(error) => RenderState::unavailable(error),
            }
        } else {
            self.state(&cursor.id).await
        }
    }
    pub(crate) async fn remove(&mut self, id: &str) {
        if self.process.is_some() {
            let _ = self.request(Request::Remove { id: id.into() }).await;
        }
    }
    pub(crate) async fn stop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.input.shutdown().await;
            drop(process.input);
            if tokio::time::timeout(Duration::from_secs(2), process.child.wait())
                .await
                .is_err()
            {
                let _ = process.child.kill().await;
                let _ = process.child.wait().await;
            }
        }
    }
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

impl crate::protocol::Action {
    pub(crate) fn cursor_target(&self) -> Option<crate::protocol::Position> {
        use crate::protocol::{Action, Position};
        match self {
            Action::MoveCursor { target }
            | Action::Click { target, .. }
            | Action::Scroll { target, .. } => Some(target.clone()),
            Action::Drag { from, .. } => Some(Position::Point(*from)),
            Action::SetValue { index, .. }
            | Action::SelectText { index, .. }
            | Action::Secondary { index, .. } => Some(Position::Element(*index)),
            Action::TypeText { index, .. }
            | Action::Paste { index, .. }
            | Action::PressKey { index, .. } => index.map(Position::Element),
            _ => None,
        }
    }
    pub(crate) fn cursor_action(&self) -> cursor_overlay::CursorAction {
        use crate::protocol::Action;
        use cursor_overlay::CursorAction as Cue;
        match self {
            Action::MoveCursor { .. } => Cue::Idle,
            Action::Click { .. } | Action::Secondary { .. } => Cue::Click,
            Action::Drag { .. } => Cue::Drag,
            Action::Scroll { .. } => Cue::Scroll,
            Action::TypeText { .. }
            | Action::Paste { .. }
            | Action::SetValue { .. }
            | Action::SelectText { .. } => Cue::Text,
            Action::PressKey { .. } => Cue::Key,
            Action::Navigate { .. } | Action::Back | Action::Forward | Action::Reload => {
                Cue::Navigate
            }
            Action::Close => Cue::App,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires MAKA_CUA_TEST_HOST; starts an invisible native display helper without input"]
    async fn renderer_loads_maka_artifact_and_exits_with_its_channel() {
        let executable =
            std::env::var_os("MAKA_CUA_TEST_HOST").expect("set the built maka executable");
        let mut host = Host::with_executable(executable.clone().into());
        let cursor = Spec {
            enabled: false,
            color: Color::Blue,
            ..Default::default()
        };
        let state = host
            .update(&cursor, None, cursor_overlay::CursorAction::Idle, None)
            .await
            .unwrap();
        assert_eq!(state.theme.as_deref(), Some(cursor.theme_id().as_str()));
        assert!(!state.visible);
        let pid = state.renderer_pid.unwrap();
        host.stop().await;
        #[cfg(unix)]
        assert_ne!(
            unsafe { libc::kill(pid as i32, 0) },
            0,
            "renderer must exit after EOF"
        );
        #[cfg(not(unix))]
        let _ = pid;

        // A broken response pipe must not strand the AppKit main loop after
        // its control thread encounters an I/O error.
        let mut child = tokio::process::Command::new(executable)
            .arg(ENTRY)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let mut request = serde_json::to_vec(&Request::State { id: cursor.id }).unwrap();
        request.push(b'\n');
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&request)
            .await
            .unwrap();
        let exit = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .expect("broken pipe must stop the renderer")
            .unwrap();
        assert!(!exit.success());
    }
}
