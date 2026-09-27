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

use crate::{driver::Driver, protocol::*};
use cua_driver_contract::{ListAppsOutput, ListWindowsOutput, WindowInfo, WindowStateOutput};
use maka_runtime::{
    capability::{CallResult, ContentBlock},
    tools::ToolError,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

/// Native targets and observation identities have one Session owner. REPL
/// objects contain only random handles; the Host supplies current admission.
pub struct Session {
    native: std::sync::Arc<tokio::sync::Mutex<Driver>>,
    targets: HashMap<String, Target>,
    id: Option<String>,
    closed: bool,
    browsers: crate::browser::Browsers,
}
struct Target {
    #[cfg(target_os = "macos")]
    macos: Option<std::sync::Arc<std::sync::Mutex<crate::macos::Target>>>,
    window: WindowInfo,
    process: String,
    snapshot: Option<WindowStateOutput>,
    capture: Option<String>,
    previous: Option<String>,
}
impl Session {
    pub fn new(native: std::sync::Arc<tokio::sync::Mutex<Driver>>) -> Self {
        Self {
            native,
            targets: HashMap::new(),
            id: None,
            closed: false,
            browsers: Default::default(),
        }
    }
    pub fn configure_browsers(
        &mut self,
        connections: Vec<maka_plugins::computer::BrowserConnection>,
    ) -> Result<(), ToolError> {
        self.browsers.configure(connections)
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
        if self.id.as_deref().is_some_and(|id| id != session) {
            return Err(failed("Computer Use Session identity mismatch"));
        }
        self.id.get_or_insert_with(|| session.into());
        match command {
            Command::Documentation => Ok(json!(include_str!("api.md"))),
            Command::ListApps { .. } => self.apps(session, cancellation).await,
            Command::ListWindows { .. } => Ok(json!(
                self.windows(session, cancellation)
                    .await?
                    .iter()
                    .map(window_info)
                    .collect::<Vec<_>>()
            )),
            Command::GetState { .. } => {
                let (browsers, mut errors) = self.browsers.state().await;
                #[cfg(target_os = "macos")]
                if let Err(error) = crate::macos::require_desktop() {
                    errors.push(error.to_string());
                }
                let apps = match self.apps(session, cancellation).await {
                    Ok(apps) => apps,
                    Err(error) => {
                        errors.push(error.to_string());
                        json!([])
                    }
                };
                Ok(json!({"apps": apps, "browsers": browsers, "errors":errors}))
            }
            Command::GetApp { target } => {
                #[cfg(target_os = "macos")]
                crate::macos::require_desktop()?;
                if self.targets.len() >= 64 {
                    return Err(failed(
                        "target limit reached; reset Cua to release bindings",
                    ));
                }
                let windows = self.windows(session, cancellation).await?;
                let candidates: Vec<_> = match target {
                    AppReference::Window { window_id } => windows
                        .into_iter()
                        .filter(|w| w.window_id == window_id)
                        .collect(),
                    AppReference::Name(name) => {
                        let apps: ListAppsOutput = decode(
                            self.native
                                .lock()
                                .await
                                .invoke("list_apps", json!({}), session, cancellation)
                                .await?,
                        )?;
                        let pids: Vec<_> = apps
                            .apps
                            .iter()
                            .filter(|a| {
                                a.name.eq_ignore_ascii_case(&name)
                                    || a.bundle_id.as_deref() == Some(&name)
                                    || a.launch_path.as_deref() == Some(&name)
                            })
                            .map(|a| a.pid)
                            .collect();
                        let selected: Vec<_> = windows
                            .into_iter()
                            .filter(|w| w.pid.is_some_and(|pid| pids.contains(&pid)))
                            .collect();
                        if selected.is_empty() && cfg!(target_os = "macos") {
                            let bundle = name.contains('.')
                                && !name.contains('/')
                                && !name.contains(' ')
                                && !name.ends_with(".app")
                                && name.chars().all(|c| {
                                    c.is_ascii_lowercase()
                                        || c.is_ascii_digit()
                                        || matches!(c, '.' | '-')
                                });
                            let args = if bundle {
                                json!({"bundle_id":name})
                            } else {
                                json!({"name":name})
                            };
                            let launched = self
                                .native
                                .lock()
                                .await
                                .invoke("launch_app", args, session, cancellation)
                                .await?;
                            let pid=launched.structured_content.as_ref().and_then(|value| value["pid"].as_u64()).ok_or_else(|| ToolError::OutcomeUnknown("launch requested but process identity unavailable; inspect inventory before retrying".into()))?;
                            self.windows(session, cancellation)
                                .await?
                                .into_iter()
                                .filter(|w| w.pid.map(u64::from) == Some(pid))
                                .collect()
                        } else {
                            selected
                        }
                    }
                };
                if candidates.len() != 1 {
                    return Err(failed(format!(
                        "App selection requires exactly one open window; found {}. Choose cua.getApp({{windowId}}) from {}",
                        candidates.len(),
                        json!(candidates.iter().map(window_info).collect::<Vec<_>>())
                    )));
                }
                let window = candidates.into_iter().next().unwrap();
                let pid = window
                    .pid
                    .ok_or_else(|| failed("window process identity unavailable"))?;
                let process = crate::identity::process(pid)?;
                #[cfg(target_os = "macos")]
                let macos = {
                    let id = window.window_id;
                    tokio::task::spawn_blocking(move || crate::macos::Target::bind(pid, id))
                        .await
                        .map_err(failed)?
                        .ok()
                        .map(|target| std::sync::Arc::new(std::sync::Mutex::new(target)))
                };
                let handle = uuid::Uuid::new_v4().to_string();
                self.targets.insert(
                    handle.clone(),
                    Target {
                        #[cfg(target_os = "macos")]
                        macos,
                        window,
                        process,
                        snapshot: None,
                        capture: None,
                        previous: None,
                    },
                );
                let state = match self
                    .observe(&handle, ObservationKind::Ax, false, session, cancellation)
                    .await
                {
                    Ok(value) => value,
                    Err(error) => {
                        self.targets.remove(&handle);
                        return Err(error);
                    }
                };
                let bound = self
                    .targets
                    .get(&handle)
                    .expect("observed target remains bound");
                Ok(
                    json!({"handle":Handle::App(handle.clone()),"kind":"app","state":state["state"],"binding":{"pid":bound.window.pid,"windowId":bound.window.window_id,"processGeneration":bound.process}}),
                )
            }
            Command::Observe {
                handle: Handle::App(handle),
                kind,
                options,
            } => {
                self.observe(
                    &handle,
                    kind,
                    !options.disable_diffing,
                    session,
                    cancellation,
                )
                .await
            }
            Command::Action {
                handle: Handle::App(handle),
                action,
            } => self.action(&handle, action, session, cancellation).await,
            command => self.browsers.invoke(command).await,
        }
    }
    async fn windows(
        &mut self,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<WindowInfo>, ToolError> {
        let output: ListWindowsOutput = decode(
            self.native
                .lock()
                .await
                .invoke("list_windows", json!({}), session, cancellation)
                .await?,
        )?;
        Ok(output.windows)
    }
    async fn apps(
        &mut self,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<Value, ToolError> {
        let apps: ListAppsOutput = decode(
            self.native
                .lock()
                .await
                .invoke("list_apps", json!({}), session, cancellation)
                .await?,
        )?;
        let windows = self.windows(session, cancellation).await?;
        Ok(json!(apps.apps.into_iter().map(|app| json!({
            "id":app.bundle_id.unwrap_or_else(|| app.pid.to_string()), "displayName":app.name,
            "isRunning":app.running, "windows":windows.iter().filter(|w| w.pid == Some(app.pid)).map(window_info).collect::<Vec<_>>()
        })).collect::<Vec<_>>()))
    }
    async fn verify(
        &mut self,
        handle: &str,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<WindowInfo, ToolError> {
        #[cfg(target_os = "macos")]
        crate::macos::require_desktop()?;
        let target = self
            .targets
            .get(handle)
            .ok_or_else(|| failed("stale_target: bind the app again"))?;
        let pid = target
            .window
            .pid
            .ok_or_else(|| failed("window has no process"))?;
        if crate::identity::process(pid).ok().as_ref() != Some(&target.process) {
            self.targets.remove(handle);
            return Err(failed(
                "stale_target: application process ended or changed; bind it again",
            ));
        }
        let id = target.window.window_id;
        #[cfg(target_os = "macos")]
        let geometry = if let Some(macos) = target.macos.clone() {
            let verified = tokio::task::spawn_blocking(move || {
                let target = macos.lock().unwrap();
                target.verify()?;
                target.geometry()
            })
            .await
            .map_err(failed)?;
            match verified {
                Ok(bounds) => Some(bounds),
                Err(error) => {
                    self.targets.remove(handle);
                    return Err(error);
                }
            }
        } else {
            None
        };
        let windows = self.windows(session, cancellation).await?;
        if let Some(window) = windows
            .into_iter()
            .find(|w| w.pid == Some(pid) && w.window_id == id)
        {
            #[cfg(target_os = "macos")]
            let window = if let Some(bounds) = geometry {
                WindowInfo { bounds, ..window }
            } else {
                window
            };
            return Ok(window);
        }
        self.targets.remove(handle);
        Err(failed("stale_target: window closed; bind a current window"))
    }
    async fn observe(
        &mut self,
        handle: &str,
        kind: ObservationKind,
        diff: bool,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<Value, ToolError> {
        let window = self.verify(handle, session, cancellation).await?;
        let ax = !matches!(kind, ObservationKind::Screenshot);
        let screenshot = !matches!(kind, ObservationKind::Ax);
        // Clear old authority before attempting observation; failure cannot leave
        // a misleading older capture active under a newer model observation.
        let target = self.targets.get_mut(handle).unwrap();
        target.snapshot = None;
        target.capture = None;
        #[cfg(target_os = "macos")]
        let own_ax = target.macos.is_some();
        #[cfg(not(target_os = "macos"))]
        let own_ax = false;
        #[cfg(target_os = "macos")]
        if let Some(macos) = target.macos.clone() {
            macos.lock().unwrap().clear();
        }
        let result = if !own_ax || screenshot {
            self.native.lock().await.invoke("get_window_state", json!({"pid":window.pid,"window_id":window.window_id,"include_accessibility_tree":ax && !own_ax,"include_screenshot":screenshot,"max_elements":1000,"max_image_dimension":1600}), session, cancellation).await?
        } else {
            CallResult {
                content: vec![],
                structured_content: Some(json!({"pid":window.pid,"window_id":window.window_id})),
            }
        };
        let capture = result
            .structured_content
            .as_ref()
            .and_then(|v| v["capture_id"].as_str())
            .map(str::to_owned);
        let image = result
            .content
            .iter()
            .find(|block| matches!(block, ContentBlock::Image { .. }))
            .cloned();
        let snapshot: WindowStateOutput = decode(result)?;
        if snapshot.pid != window.pid.unwrap() || snapshot.window_id != window.window_id {
            return Err(failed("observation target changed"));
        }
        if screenshot && image.is_none() {
            return Err(failed("screenshot_unavailable: provider returned no image"));
        }
        if screenshot && self.verify(handle, session, cancellation).await?.bounds != window.bounds {
            return Err(failed(
                "window geometry changed during capture; capture again",
            ));
        }
        let target = self.targets.get_mut(handle).unwrap();
        let mut output = serde_json::Map::new();
        if ax {
            let full = snapshot.tree_markdown.clone().unwrap_or_default();
            #[cfg(target_os = "macos")]
            let full = if let Some(macos) = target.macos.clone() {
                tokio::task::spawn_blocking(move || macos.lock().unwrap().observe())
                    .await
                    .map_err(failed)??
            } else {
                full
            };
            let full = format!(
                "Window: {:?}, App: {:?}.\n{full}",
                window.title, window.app_name
            );
            let mut state = if diff {
                display_diff(target.previous.as_deref(), &full)
            } else {
                full.clone()
            };
            if snapshot.truncated == Some(true) || snapshot.degraded == Some(true) {
                state.push_str(&format!(
                    "\nObservation incomplete: {}",
                    snapshot
                        .truncation_reason
                        .as_deref()
                        .or(snapshot.degraded_reason.as_deref())
                        .unwrap_or("partial accessibility tree")
                ));
            }
            target.previous = Some(full);
            output.insert("state".into(), json!(state));
        } else {
            target.previous = None;
        }
        if let Some(image) = image {
            output.insert("image".into(), json!(image));
        }
        target.window = window;
        target.snapshot = Some(snapshot);
        target.capture = capture;
        Ok(Value::Object(output))
    }
    async fn action(
        &mut self,
        handle: &str,
        action: Action,
        session: &str,
        cancellation: &CancellationToken,
    ) -> Result<Value, ToolError> {
        let window = self.verify(handle, session, cancellation).await?;
        let target = self.targets.get(handle).unwrap();
        #[cfg(target_os = "macos")]
        if crate::macos::Target::handles(&action)
            && let Some(macos) = target.macos.clone()
        {
            let result = tokio::task::spawn_blocking(move || macos.lock().unwrap().action(action))
                .await
                .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
            self.targets.get_mut(handle).unwrap().capture = None;
            return result.map(|()| Value::Null);
        }
        let bound = json!({"kind":"window","pid":window.pid,"window_id":window.window_id});
        #[cfg(target_os = "macos")]
        if let Action::Paste {
            index: None,
            text,
            options,
        } = action
        {
            let html = matches!(options.format, Some(PasteFormat::Html));
            let macos = target
                .macos
                .clone()
                .ok_or_else(|| failed("paste requires an exact native accessibility target"))?;
            let result =
                tokio::task::spawn_blocking(move || macos.lock().unwrap().paste(&text, html))
                    .await
                    .map_err(|error| ToolError::CleanupUnconfirmed(error.to_string()))?;
            self.targets.get_mut(handle).unwrap().capture = None;
            let restored = result?;
            return Ok(if restored {
                Value::Null
            } else {
                json!({"warning":"Clipboard changed during paste; preserved the newer clipboard contents."})
            });
        }
        let (name, args) = match action {
            Action::Click {
                target: position,
                options,
            } => {
                let count = options.click_count.unwrap_or(1);
                if !(1..=3).contains(&count) {
                    return Err(failed("clickCount must be between 1 and 3"));
                }
                let mut args = json!({"target":bound,"button":options.mouse_button.unwrap_or(MouseButton::Left).name(),"count":count,"delivery_mode":if cfg!(target_os="windows") {"foreground"} else {"background"}});
                match position {
                    Position::Element(index) => {
                        args["element_token"] = json!(target.element(index)?.element_token)
                    }
                    Position::Point(point) => {
                        target.point(point, &window)?;
                        args["x"] = json!(point[0]);
                        args["y"] = json!(point[1]);
                        args["capture_id"] = json!(target.capture);
                    }
                }
                ("click", args)
            }
            Action::SetValue { index, value } => (
                "set_value",
                json!({"pid":window.pid,"window_id":window.window_id,"element_token":target.element(index)?.element_token,"value":value}),
            ),
            Action::Secondary { index, action } if cfg!(target_os = "macos") => {
                let element = target.element(index)?;
                let (native, name) = match action.as_str() {
                    "AXShowMenu" | "Show Menu" => ("AXShowMenu", "show_menu"),
                    "AXPick" | "Pick" => ("AXPick", "pick"),
                    "AXConfirm" | "Confirm" => ("AXConfirm", "confirm"),
                    "AXCancel" | "Cancel" => ("AXCancel", "cancel"),
                    "AXOpen" | "Open" => ("AXOpen", "open"),
                    _ => {
                        return Err(failed(
                            "unsupported: Cua cannot deliver this accessibility action",
                        ));
                    }
                };
                if !element
                    .actions
                    .as_ref()
                    .is_some_and(|actions| actions.iter().any(|a| a == native))
                {
                    return Err(failed(
                        "action_not_exposed: select an action from the current accessibility observation",
                    ));
                }
                (
                    "click",
                    json!({"pid":window.pid,"window_id":window.window_id,"element_token":element.element_token,"action":name,"delivery_mode":"background"}),
                )
            }
            Action::TypeText { index: None, text } => {
                ("type_text", json!({"target":bound,"text":text}))
            }
            Action::Paste {
                index: None,
                text,
                options,
            } if matches!(options.format, None | Some(PasteFormat::Text)) => {
                ("type_text", json!({"target":bound,"text":text}))
            }
            Action::PressKey { index: None, key } => {
                let mut parts: Vec<_> = key.split('+').collect();
                let key = parts.pop().ok_or_else(|| failed("key must not be empty"))?;
                if key.is_empty() {
                    return Err(failed("key must not be empty"));
                }
                let modifiers: Result<Vec<_>, _> = parts
                    .into_iter()
                    .map(|part| match part.to_ascii_lowercase().as_str() {
                        "super" | "meta" | "cmd" | "command" => Ok(if cfg!(target_os = "macos") {
                            "cmd"
                        } else {
                            "super"
                        }),
                        "ctrl" | "control" => Ok("ctrl"),
                        "alt" | "option" => Ok("alt"),
                        "shift" => Ok("shift"),
                        _ => Err(failed("unsupported key modifier")),
                    })
                    .collect();
                (
                    "press_key",
                    json!({"target":bound,"key":key,"modifiers":modifiers?}),
                )
            }
            Action::Scroll {
                target: position,
                direction,
                distance,
            } => {
                let amount = match distance {
                    None => 1,
                    Some(Distance::Pages(pages)) => pages,
                    Some(Distance::Pixels { .. }) => {
                        return Err(failed(
                            "unsupported: this native adapter scrolls in pages, not exact pixels",
                        ));
                    }
                };
                if amount == 0 || amount > 50 {
                    return Err(failed("scroll distance must be between 1 and 50 pages"));
                }
                let mut args = json!({"direction":direction.name(),"by":"page","amount":amount});
                match position {
                    Position::Point(point) => {
                        target.point(point, &window)?;
                        args["target"] = bound;
                        args["x"] = json!(point[0]);
                        args["y"] = json!(point[1]);
                    }
                    Position::Element(index) if cfg!(target_os = "macos") => {
                        args["pid"] = json!(window.pid);
                        args["window_id"] = json!(window.window_id);
                        args["element_token"] = json!(target.element(index)?.element_token);
                        args["delivery_mode"] = json!("background");
                    }
                    _ => {
                        return Err(failed(
                            "unsupported: element scrolling is unavailable on this platform",
                        ));
                    }
                }
                ("scroll", args)
            }
            Action::Drag { from, to } => {
                target.point(from, &window)?;
                target.point(to, &window)?;
                (
                    "drag",
                    json!({"target":bound,"from_x":from[0],"from_y":from[1],"to_x":to[0],"to_y":to[1]}),
                )
            }
            _ => {
                return Err(failed(
                    "unsupported: this native operation is not implemented by the current adapter",
                ));
            }
        };
        let result = self
            .native
            .lock()
            .await
            .invoke(name, args, session, cancellation)
            .await;
        // Every attempt invalidates coordinates. Element tokens stay bound to
        // the retained native nodes and are revalidated by Cua on each action.
        self.targets.get_mut(handle).unwrap().capture = None;
        result.map(|result| result.structured_content.unwrap_or(Value::Null))
    }
    pub async fn close(&mut self) -> Result<(), ToolError> {
        self.closed = true;
        self.targets.clear();
        self.browsers.close();
        if let Some(session) = &self.id {
            self.native.lock().await.release_session(session).await?;
        }
        self.id = None;
        Ok(())
    }
}
impl Target {
    fn element(&self, index: u64) -> Result<&cua_driver_contract::WindowElement, ToolError> {
        self.snapshot
            .as_ref()
            .and_then(|s| s.elements.as_ref())
            .and_then(|elements| {
                elements
                    .iter()
                    .find(|e| e.element_index == index && e.element_token.is_some())
            })
            .ok_or_else(|| failed("stale_element: read getAXState before using this index"))
    }
    fn point(&self, point: [f64; 2], window: &WindowInfo) -> Result<(), ToolError> {
        let snapshot = self.snapshot.as_ref().ok_or_else(|| {
            failed("capture_required: read getScreenshot before coordinate input")
        })?;
        if self.capture.is_none()
            || self.window.bounds != window.bounds
            || snapshot.screenshot_frame_valid == Some(false)
        {
            return Err(failed(
                "stale_capture: window changed or capture was consumed; getScreenshot again",
            ));
        }
        if !point.iter().all(|v| v.is_finite() && *v >= 0.0)
            || point[0] >= f64::from(snapshot.screenshot_width.unwrap_or(0))
            || point[1] >= f64::from(snapshot.screenshot_height.unwrap_or(0))
        {
            return Err(failed("coordinate outside the captured image"));
        }
        Ok(())
    }
}
fn window_info(window: &WindowInfo) -> Value {
    json!({"id":window.window_id,"app":window.app_name,"title":window.title})
}
fn decode<T: DeserializeOwned>(result: CallResult) -> Result<T, ToolError> {
    serde_json::from_value(
        result
            .structured_content
            .ok_or_else(|| failed("native structured output unavailable"))?,
    )
    .map_err(|error| ToolError::OutcomeUnknown(format!("invalid native result: {error}")))
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
pub(crate) fn display_diff(previous: Option<&str>, current: &str) -> String {
    let Some(previous) = previous else {
        return current.into();
    };
    if previous == current {
        return "Accessibility state unchanged.".into();
    }
    let old: std::collections::HashSet<_> = previous.lines().collect();
    let new: std::collections::HashSet<_> = current.lines().collect();
    let changes: Vec<_> = previous
        .lines()
        .filter(|line| !new.contains(line))
        .map(|line| format!("- {line}"))
        .chain(
            current
                .lines()
                .filter(|line| !old.contains(line))
                .map(|line| format!("+ {line}")),
        )
        .collect();
    let diff = format!("Accessibility changes:\n{}", changes.join("\n"));
    if changes.is_empty() || diff.len() >= current.len() {
        current.into()
    } else {
        diff
    }
}
