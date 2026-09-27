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

//! Private, parent-owned desktop worker. There is no listener or reconnect path.
use super::{failed, local_platform};
use crate::{Driver, protocol::Command, session::Session};
use maka_plugins::computer::BrowserConnection;
use maka_runtime::tools::ToolError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    process::{ExitCode, Stdio},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;

const ENTRY: &str = "__maka-cua-desktop";
const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 16 * 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    protocol: u32,
    platform: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Invoke {
        session: String,
        browsers: Vec<BrowserConnection>,
        command: Command,
    },
    CloseSession {
        session: String,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
enum Failure {
    Failed(String),
    OutcomeUnknown(String),
    CleanupUnconfirmed(String),
}
impl From<ToolError> for Failure {
    fn from(error: ToolError) -> Self {
        match error {
            ToolError::Failed(message) => Self::Failed(message),
            ToolError::OutcomeUnknown(message) => Self::OutcomeUnknown(message),
            ToolError::CleanupUnconfirmed(message) | ToolError::Persistence(message) => {
                Self::CleanupUnconfirmed(message)
            }
            ToolError::Io { message, .. } => Self::Failed(message),
        }
    }
}
impl From<Failure> for ToolError {
    fn from(error: Failure) -> Self {
        match error {
            Failure::Failed(message) => Self::Failed(message),
            Failure::OutcomeUnknown(message) => Self::OutcomeUnknown(message),
            Failure::CleanupUnconfirmed(message) => Self::CleanupUnconfirmed(message),
        }
    }
}
type Response = Result<Value, Failure>;

#[derive(Default)]
pub(crate) struct Worker {
    process: Option<Process>,
    fault: Option<ToolError>,
    closed: bool,
}
struct Process {
    child: Child,
    pipes: Option<(ChildStdin, BufReader<ChildStdout>)>,
}
impl Worker {
    pub(crate) async fn prepare(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), ToolError> {
        if self.closed {
            return Err(failed("Windows desktop worker is closed"));
        }
        if let Some(error) = &self.fault {
            return Err(error.clone());
        }
        if self.process.is_none() {
            let executable = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(failed("Computer Use preparation cancelled")),
                executable = windows_executable() => executable?,
            };
            // The worker only advertises its protocol here. Native Cua state
            // remains idle until a separately admitted operation is sent.
            self.process = Some(Process::start(executable).await?);
        }
        if cancellation.is_cancelled() {
            return Err(failed("Computer Use preparation cancelled"));
        }
        Ok(())
    }
    pub(crate) async fn release_session(&mut self, session: &str) -> Result<(), ToolError> {
        if self.process.is_some() {
            self.call(
                Request::CloseSession {
                    session: session.into(),
                },
                &CancellationToken::new(),
            )
            .await?;
        }
        Ok(())
    }
    pub(crate) async fn call(
        &mut self,
        request: Request,
        cancellation: &CancellationToken,
    ) -> Result<Value, ToolError> {
        if cancellation.is_cancelled() {
            return Err(failed("Computer Use cancelled before dispatch"));
        }
        let frame = encode(&request).map_err(failed)?;
        self.prepare(cancellation).await?;
        let process = self.process.as_mut().unwrap();
        // Once a frame is sent, wait for settlement. A cancelled REPL must not
        // drop native input or infer that an unacknowledged action did not run.
        match process.exchange(&frame).await {
            Ok(response) => response.map_err(Into::into),
            Err(error) => {
                let message = format!(
                    "Windows desktop connection lost: {error}; restart the Host before using Computer Use again"
                );
                let result = match process.disconnect().await {
                    Ok(()) => ToolError::OutcomeUnknown(message.clone()),
                    Err(cleanup) => ToolError::CleanupUnconfirmed(format!("{message}; {cleanup}")),
                };
                self.fault = Some(match &result {
                    ToolError::OutcomeUnknown(_) => failed(&message),
                    _ => result.clone(),
                });
                Err(result)
            }
        }
    }

    pub(crate) async fn close(&mut self) -> Result<(), ToolError> {
        let mut failure = None;
        if !self.closed
            && self.process.is_some()
            && self.fault.is_none()
            && let Err(error) = self
                .call(Request::Shutdown, &CancellationToken::new())
                .await
        {
            failure = Some(error);
        }
        self.closed = true;
        if let Some(process) = &mut self.process {
            process
                .disconnect()
                .await
                .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
        }
        self.process = None;
        failure.map_or(Ok(()), Err)
    }
}
impl Process {
    async fn start(executable: PathBuf) -> Result<Self, ToolError> {
        let mut command = tokio::process::Command::new(executable);
        command
            .arg(ENTRY)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| failed(format!(
            "Windows desktop unavailable: {error}; install Windows Maka and enable WSL interoperability"
        )))?;
        Self {
            pipes: Some((
                child.stdin.take().unwrap(),
                BufReader::new(child.stdout.take().unwrap()),
            )),
            child,
        }
        .handshake()
        .await
    }

    async fn handshake(mut self) -> Result<Self, ToolError> {
        let hello = tokio::time::timeout(
            STARTUP_TIMEOUT,
            read::<_, Hello>(&mut self.pipes.as_mut().unwrap().1),
        )
        .await;
        let problem = match hello {
            Ok(Ok(Hello {
                protocol: PROTOCOL,
                platform,
            })) if platform == "windows" => return Ok(self),
            Ok(Ok(_)) => {
                "incompatible desktop worker; install a matching Windows Maka build".to_owned()
            }
            Ok(Err(error)) => format!("Windows desktop worker handshake failed: {error}"),
            Err(_) => "Windows desktop worker startup timed out".to_owned(),
        };
        self.disconnect()
            .await
            .map_err(|error| ToolError::CleanupUnconfirmed(format!("{problem}; {error}")))?;
        Err(failed(problem))
    }

    async fn exchange(&mut self, frame: &[u8]) -> io::Result<Response> {
        let (input, output) = self
            .pipes
            .as_mut()
            .ok_or_else(|| io::Error::other("desktop channel closed"))?;
        input.write_all(frame).await?;
        input.flush().await?;
        read(output).await
    }

    async fn disconnect(&mut self) -> io::Result<()> {
        // Close both directions so a peer writing a failed response cannot
        // stay blocked on its output while we wait for its cleanup.
        self.pipes.take();
        // WSL's interop process represents a Windows process, so killing the
        // Linux PID alone does not prove that native work has stopped.
        tokio::time::timeout(EXIT_TIMEOUT, self.child.wait())
            .await
            .map_err(|_| io::Error::other("Windows desktop worker exit was not confirmed"))??;
        Ok(())
    }
}

async fn windows_executable() -> Result<PathBuf, ToolError> {
    if let Some(path) = std::env::var_os("MAKA_CUA_WINDOWS_EXECUTABLE") {
        let path = PathBuf::from(path);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err(failed(
            "MAKA_CUA_WINDOWS_EXECUTABLE must be an absolute path to Windows maka.exe, as visible from WSL",
        ));
    }
    let resolve = super::WINDOWS_HELPER.get().ok_or_else(|| {
        failed("the embedding application has not configured Windows desktop provisioning")
    })?;
    resolve().await
}

/// Invoke after cursor bootstrap and before starting the embedding application.
pub fn bootstrap() -> Option<ExitCode> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ENTRY)) {
        return None;
    }
    let result = if std::env::args_os().len() != 2 {
        Err(io::Error::other("desktop worker takes no arguments"))
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .and_then(|runtime| runtime.block_on(serve_stdio()))
    };
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Maka desktop: {error}");
            ExitCode::FAILURE
        }
    })
}

async fn serve_stdio() -> io::Result<()> {
    let (send, receive) = mpsc::channel(1);
    let disconnected = CancellationToken::new();
    let eof = disconnected.clone();
    // A detached blocking reader avoids Tokio stdin keeping runtime shutdown
    // alive when stdout closes while the parent still holds stdin open.
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        while let Ok(request) = read_sync(&mut input) {
            if send.blocking_send(request).is_err() {
                break;
            }
        }
        eof.cancel();
    });
    let driver = Arc::new(Mutex::new(Driver::default()));
    serve(receive, tokio::io::stdout(), disconnected, driver).await
}

async fn serve(
    mut receive: mpsc::Receiver<Request>,
    mut output: impl AsyncWrite + Unpin,
    disconnected: CancellationToken,
    driver: Arc<Mutex<Driver>>,
) -> io::Result<()> {
    let mut sessions = HashMap::<String, Session>::new();
    write(
        &mut output,
        &Hello {
            protocol: PROTOCOL,
            platform: local_platform().into(),
        },
    )
    .await?;
    let mut shutdown = false;
    let mut transport_error = None;
    while let Some(request) = tokio::select! {
        biased;
        _ = disconnected.cancelled() => None,
        request = receive.recv() => request,
    } {
        let result = match request {
            Request::Invoke {
                session,
                browsers,
                command,
            } => {
                if session.is_empty() || session.len() > 256 {
                    Err(failed("invalid desktop Session identity"))
                } else if sessions.len() >= 16 && !sessions.contains_key(&session) {
                    Err(failed("desktop Session capacity reached"))
                } else {
                    let local = sessions
                        .entry(session.clone())
                        .or_insert_with(|| Session::new(driver.clone()));
                    match local.configure_browsers(browsers) {
                        Ok(()) => local.invoke(command, &session, &disconnected).await,
                        Err(error) => Err(error),
                    }
                }
            }
            Request::CloseSession { session } => {
                let result = match sessions.get_mut(&session) {
                    Some(local) => local.close().await,
                    None => Ok(()),
                };
                if result.is_ok() {
                    sessions.remove(&session);
                }
                result.map(|()| Value::Null)
            }
            Request::Shutdown => {
                shutdown = true;
                break;
            }
        };
        if let Err(error) = write(&mut output, &result.map_err(Failure::from)).await {
            transport_error = Some(error);
            break;
        }
    }
    let mut cleanup = Ok(Value::Null);
    for local in sessions.values_mut() {
        if let Err(error) = local.close().await {
            cleanup = Err(Failure::from(error));
        }
    }
    if let Err(error) = driver.lock().await.close().await {
        cleanup = Err(Failure::from(error));
    }
    if shutdown {
        write(&mut output, &cleanup).await?;
    }
    if let Some(error) = transport_error {
        return Err(error);
    }
    cleanup
        .map(|_| ())
        .map_err(|error| io::Error::other(format!("{error:?}")))
}

fn length(length: u32) -> io::Result<usize> {
    let length = length as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid desktop frame length",
        ));
    }
    Ok(length)
}
fn encode(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; 4];
    serde_json::to_writer(&mut bytes, value)?;
    let size = bytes.len() - 4;
    if size > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "desktop message too large",
        ));
    }
    bytes[..4].copy_from_slice(&(size as u32).to_be_bytes());
    Ok(bytes)
}
async fn write(output: &mut (impl AsyncWrite + Unpin), value: &impl Serialize) -> io::Result<()> {
    output.write_all(&encode(value)?).await?;
    output.flush().await
}
async fn read<R: AsyncRead + Unpin, T: DeserializeOwned>(input: &mut R) -> io::Result<T> {
    let mut bytes = vec![0; length(input.read_u32().await?)?];
    input.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}
fn read_sync<T: DeserializeOwned>(input: &mut impl io::Read) -> io::Result<T> {
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let mut bytes = vec![0; length(u32::from_be_bytes(header))?];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(session: &str, command: Value) -> Request {
        Request::Invoke {
            session: session.into(),
            browsers: Vec::new(),
            command: serde_json::from_value(command).unwrap(),
        }
    }

    #[tokio::test]
    async fn desktop_sessions_own_targets_and_reset_independently() {
        let (send, receive) = mpsc::channel(1);
        let (mut output, writer) = tokio::io::duplex(64 * 1024);
        let driver = Arc::new(Mutex::new(Driver::default()));
        let task = tokio::spawn(serve(receive, writer, CancellationToken::new(), driver));
        let hello: Hello = read(&mut output).await.unwrap();
        assert_eq!(hello.platform, local_platform());
        for (session, label) in [("a", "Alice"), ("b", "Bob")] {
            send.send(request(
                session,
                json!({"method":"configureCursor","options":{"label":label,"enabled":false}}),
            ))
            .await
            .unwrap();
            let result: Response = read(&mut output).await.unwrap();
            assert_eq!(result.unwrap()["settings"]["label"], label);
        }
        send.send(Request::CloseSession {
            session: "a".into(),
        })
        .await
        .unwrap();
        assert!(read::<_, Response>(&mut output).await.unwrap().is_ok());
        for (session, label) in [("a", "Maka"), ("b", "Bob")] {
            send.send(request(
                session,
                json!({"method":"cursorState","options":{}}),
            ))
            .await
            .unwrap();
            let result: Response = read(&mut output).await.unwrap();
            assert_eq!(result.unwrap()["settings"]["label"], label);
        }
        send.send(request("a", json!({"method":"observe","handle":{"kind":"app","id":"unbound"},"kind":"ax","options":{}}))).await.unwrap();
        let error = read::<_, Response>(&mut output).await.unwrap().unwrap_err();
        assert!(
            matches!(error, Failure::Failed(message) if message == "stale_target: bind the app again")
        );
        send.send(Request::Shutdown).await.unwrap();
        assert!(read::<_, Response>(&mut output).await.unwrap().is_ok());
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn disconnected_desktop_discards_queued_work_and_closes_output() {
        let (send, receive) = mpsc::channel(1);
        send.send(request("queued", json!({"method":"documentation"})))
            .await
            .unwrap();
        let stop = CancellationToken::new();
        stop.cancel();
        let (mut output, writer) = tokio::io::duplex(1024);
        let task = tokio::spawn(serve(
            receive,
            writer,
            stop,
            Arc::new(Mutex::new(Driver::default())),
        ));
        let _: Hello = read(&mut output).await.unwrap();
        assert_eq!(
            read::<_, Response>(&mut output).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn desktop_frames_reject_oversize_and_partial_responses() {
        let mut oversize = ((MAX_FRAME + 1) as u32).to_be_bytes().as_slice().to_vec();
        assert_eq!(
            read::<_, Response>(&mut oversize.as_slice())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read_sync::<Response>(&mut oversize.as_slice())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        oversize = encode(&Ok::<_, Failure>(json!({"image":"payload"}))).unwrap();
        oversize.pop();
        assert_eq!(
            read::<_, Response>(&mut oversize.as_slice())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[cfg(unix)]
    fn child(hello: Hello) -> Process {
        let frame = encode(&hello)
            .unwrap()
            .iter()
            .map(|byte| format!("\\{:03o}", byte))
            .collect::<String>();
        // This disposable peer exits after receiving input, without acknowledging
        // it. Exercise the same real pipes and process wait as the WSL client.
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "printf '%b' '{frame}'; dd bs=1 count=1 of=/dev/null 2>/dev/null"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Process {
            pipes: Some((
                child.stdin.take().unwrap(),
                BufReader::new(child.stdout.take().unwrap()),
            )),
            child,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lost_acknowledgment_is_unknown_and_never_replayed() {
        let process = child(Hello {
            protocol: PROTOCOL,
            platform: "windows".into(),
        })
        .handshake()
        .await
        .unwrap();
        let mut worker = Worker {
            process: Some(process),
            ..Default::default()
        };
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            matches!(worker.call(request("a", json!({"method":"documentation"})), &cancelled).await,
            Err(ToolError::Failed(message)) if message == "Computer Use cancelled before dispatch")
        );
        assert!(matches!(
            worker
                .call(
                    request("a", json!({"method":"documentation"})),
                    &CancellationToken::new()
                )
                .await,
            Err(ToolError::OutcomeUnknown(_))
        ));
        assert!(
            matches!(worker.call(request("a", json!({"method":"documentation"})), &CancellationToken::new()).await, Err(ToolError::Failed(message)) if message.contains("restart the Host"))
        );
        worker.close().await.unwrap();
        for hello in [
            Hello {
                protocol: PROTOCOL + 1,
                platform: "windows".into(),
            },
            Hello {
                protocol: PROTOCOL,
                platform: "linux".into(),
            },
        ] {
            assert!(
                matches!(child(hello).handshake().await, Err(ToolError::Failed(message)) if message.contains("incompatible desktop worker"))
            );
        }
    }
}
