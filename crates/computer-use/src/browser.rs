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

use crate::protocol::*;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use maka_plugins::computer::BrowserConnection;
use maka_runtime::tools::ToolError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
mod ax;
mod cursor;

pub(crate) fn validate_connections(connections: &[BrowserConnection]) -> Result<(), ToolError> {
    let mut ids = std::collections::HashSet::new();
    if connections.len() > 8 {
        return Err(failed("at most 8 browser providers may be configured"));
    }
    for connection in connections {
        let url = reqwest::Url::parse(&connection.endpoint).map_err(failed)?;
        let loopback = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !loopback
            || url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(failed(
                "browser endpoint must be a loopback HTTP origin, without credentials, path or query",
            ));
        }
        if connection.id.is_empty() || !ids.insert(&connection.id) {
            return Err(failed("browser IDs must be nonempty and unique"));
        }
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Browsers {
    connections: Vec<BrowserConnection>,
    targets: HashMap<String, Tab>,
}
#[derive(Clone, Deserialize)]
struct TabInfo {
    id: String,
    title: String,
    url: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket: Option<String>,
}
struct Tab {
    provider: BrowserConnection,
    generation: String,
    info: TabInfo,
    socket: Option<Cdp>,
    frame: Option<String>,
    refs: HashMap<u64, u64>,
    ax: ax::Projection,
    previous: Option<String>,
    capture: Option<Capture>,
    navigation: Option<Navigation>,
    cursor_world: Option<(String, u64)>,
}
enum Navigation {
    Loader(String),
    Reload(String),
    Url(String),
}
#[derive(Clone, Copy)]
struct Capture {
    width: f64,
    height: f64,
    image_width: f64,
    image_height: f64,
    x: f64,
    y: f64,
}
struct Cdp {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sequence: u64,
}
impl Cdp {
    async fn connect(url: &str, provider: &BrowserConnection) -> Result<Self, ToolError> {
        let url = reqwest::Url::parse(url).map_err(failed)?;
        let endpoint = reqwest::Url::parse(&provider.endpoint).map_err(failed)?;
        if url.scheme() != "ws"
            || url.host_str() != endpoint.host_str()
            || url.port_or_known_default() != endpoint.port_or_known_default()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(failed(
                "browser returned a websocket outside its configured endpoint",
            ));
        }
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(16 * 1024 * 1024));
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false),
        )
        .await
        .map_err(failed)?
        .map_err(failed)?;
        Ok(Self {
            socket,
            sequence: 0,
        })
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.sequence += 1;
        let id = self.sequence;
        let request = async {
            self.socket
                .send(Message::text(
                    json!({"id":id,"method":method,"params":params}).to_string(),
                ))
                .await
                .map_err(unknown)?;
            while let Some(message) = self.socket.next().await {
                let message = message.map_err(unknown)?;
                if let Message::Text(text) = message {
                    let response: Value = serde_json::from_str(&text).map_err(unknown)?;
                    if response["id"] != id {
                        continue;
                    }
                    if let Some(error) = response.get("error") {
                        return Err(failed(format!("CDP {method}: {error}")));
                    }
                    return Ok(response["result"].clone());
                }
            }
            Err(unknown(
                "browser connection ended before acknowledging the operation",
            ))
        };
        tokio::time::timeout(Duration::from_secs(15), request)
            .await
            .map_err(|_| unknown(format!("CDP {method} timed out; do not replay a mutation")))?
    }
}
async fn http(provider: &BrowserConnection, path: &str) -> Result<Value, ToolError> {
    let url = format!("{}{path}", provider.endpoint.trim_end_matches('/'));
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(failed)?
        .get(url)
        .send()
        .await
        .map_err(failed)?
        .error_for_status()
        .map_err(failed)?;
    if response
        .content_length()
        .is_some_and(|size| size > 1024 * 1024)
    {
        return Err(failed("browser inventory exceeds limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err(failed("browser inventory exceeds limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(failed)
}
async fn inventory(provider: &BrowserConnection) -> Result<(String, Vec<TabInfo>), ToolError> {
    let version = http(provider, "/json/version").await?;
    let generation = version["webSocketDebuggerUrl"]
        .as_str()
        .ok_or_else(|| failed("browser generation unavailable"))?
        .to_owned();
    let tabs: Vec<TabInfo> =
        serde_json::from_value(http(provider, "/json/list").await?).map_err(failed)?;
    Ok((
        generation,
        tabs.into_iter().filter(|t| t.kind == "page").collect(),
    ))
}
fn tab_info(provider: &BrowserConnection, tab: &TabInfo) -> Value {
    json!({"id":format!("{}:{}",provider.id,tab.id),"providerTabId":tab.id,"browserId":provider.id,"title":tab.title,"url":tab.url})
}
impl Browsers {
    pub(crate) fn target_label(&self, id: &str) -> Option<&str> {
        self.targets.get(id).map(|tab| tab.provider.id.as_str())
    }

    pub(crate) fn configure(
        &mut self,
        connections: Vec<BrowserConnection>,
    ) -> Result<(), ToolError> {
        validate_connections(&connections)?;
        if connections != self.connections && !self.targets.is_empty() {
            return Err(failed(
                "browser configuration changed; reset Cua before using the new providers",
            ));
        }
        self.connections = connections;
        Ok(())
    }
    pub(crate) async fn state(&self) -> (Vec<Value>, Vec<String>) {
        let mut browsers = Vec::new();
        let mut errors = Vec::new();
        for provider in &self.connections {
            match inventory(provider).await {
                Ok((_, tabs)) => browsers.push(json!({"id":provider.id,"name":provider.id,"type":"cdp","tabs":tabs.iter().map(|t| tab_info(provider,t)).collect::<Vec<_>>() })),
                Err(error) => errors.push(format!("{}: {error}", provider.id)),
            }
        }
        (browsers, errors)
    }
    pub(crate) async fn invoke(
        &mut self,
        command: Command,
        cursor: &crate::cursor::Spec,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        match command {
            Command::ListBrowsers { .. } => {
                let (mut browsers, errors) = self.state().await;
                if !errors.is_empty() {
                    return Err(failed(errors.join("\n")));
                }
                for browser in &mut browsers {
                    browser.as_object_mut().unwrap().remove("tabs");
                }
                Ok(json!(browsers))
            }
            Command::ListTabs { options } => {
                let mut tabs = Vec::new();
                for provider in self.providers(options.browser.as_deref())? {
                    let (_, listed) = inventory(provider).await?;
                    tabs.extend(listed.iter().map(|tab| tab_info(provider, tab)));
                }
                Ok(json!(tabs))
            }
            Command::GetBrowser { options } => {
                if options.extension_instance_id.is_some() {
                    return Err(failed("extension browser providers are unavailable"));
                }
                let mut matches = Vec::new();
                for provider in self.providers(options.id.as_deref())? {
                    let (_, tabs) = inventory(provider).await?;
                    if options
                        .url
                        .as_ref()
                        .is_none_or(|url| tabs.iter().any(|tab| &tab.url == url))
                    {
                        matches.push(provider);
                    }
                }
                if matches.len() != 1 {
                    return Err(failed(
                        "browser selection is ambiguous or missing; select an ID from listBrowsers",
                    ));
                }
                Ok(json!({"id":matches[0].id}))
            }
            Command::GetTab { reference, options } => {
                let mut matches = Vec::new();
                for provider in self.providers(options.browser.as_deref())? {
                    let (generation, tabs) = inventory(provider).await?;
                    for tab in tabs {
                        let selected = match &reference {
                            TabReference::Id(id) => {
                                id == &tab.id || id == &format!("{}:{}", provider.id, tab.id)
                            }
                            TabReference::Url { url } => &tab.url == url,
                            TabReference::Mention { .. } => {
                                return Err(failed(
                                    "tab mentions require a connected client provider",
                                ));
                            }
                        };
                        if selected {
                            matches.push((provider.clone(), generation.clone(), tab));
                        }
                    }
                }
                if matches.len() != 1 {
                    return Err(failed(
                        "tab selection is ambiguous or missing; choose an exact ID from listTabs",
                    ));
                }
                let (provider, generation, tab) = matches.pop().unwrap();
                self.bind(provider, generation, tab, cancellation).await
            }
            Command::CreateBrowserTab {
                browser_id,
                url,
                options,
            } => {
                if options.visible.is_some() || options.session_name.is_some() {
                    return Err(failed(
                        "this CDP provider does not support visibility or sessionName options",
                    ));
                }
                validate_url(&url)?;
                let provider = self.providers(Some(&browser_id))?[0].clone();
                let (generation, _) = inventory(&provider).await?;
                let mut browser = Cdp::connect(&generation, &provider).await?;
                let created = browser
                    .request("Target.createTarget", json!({"url":url,"background":true}))
                    .await?;
                let id = created["targetId"]
                    .as_str()
                    .ok_or_else(|| unknown("browser did not return a target ID"))?;
                let (current, tabs) = inventory(&provider).await?;
                if current != generation {
                    return Err(unknown("browser restarted after tab creation"));
                }
                let tab = tabs.into_iter().find(|t| t.id == id).ok_or_else(|| {
                    unknown("created tab is not observable; do not create it again blindly")
                })?;
                self.bind(provider, generation, tab, cancellation).await
            }
            Command::Observe {
                handle: Handle::Tab(handle),
                kind,
                options,
            } => {
                self.observe(&handle, kind, !options.disable_diffing, cancellation)
                    .await
            }
            Command::Action {
                handle: Handle::Tab(handle),
                action,
            } => self.action(&handle, action, cursor, cancellation).await,
            _ => Err(failed("invalid browser operation")),
        }
    }
    fn providers(&self, id: Option<&str>) -> Result<Vec<&BrowserConnection>, ToolError> {
        let providers: Vec<_> = self
            .connections
            .iter()
            .filter(|p| id.is_none_or(|id| p.id == id))
            .collect();
        if providers.is_empty() && id.is_some() {
            return Err(failed(
                "browser_unavailable: no matching configured CDP provider",
            ));
        }
        Ok(providers)
    }
    async fn bind(
        &mut self,
        provider: BrowserConnection,
        generation: String,
        info: TabInfo,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        if self.targets.len() >= 64 {
            return Err(failed("tab binding limit reached; reset Cua"));
        }
        let handle = uuid::Uuid::new_v4().to_string();
        let id = format!("{}:{}", provider.id, info.id);
        self.targets.insert(
            handle.clone(),
            Tab {
                provider,
                generation,
                info,
                socket: None,
                frame: None,
                refs: HashMap::new(),
                ax: ax::Projection::default(),
                previous: None,
                capture: None,
                navigation: None,
                cursor_world: None,
            },
        );
        match self
            .observe(&handle, ObservationKind::Ax, false, cancellation)
            .await
        {
            Ok(value) => Ok(
                json!({"handle":Handle::Tab(handle),"id":id,"kind":"tab","state":value["state"]}),
            ),
            Err(error) => {
                self.targets.remove(&handle);
                Err(error)
            }
        }
    }
    async fn observe(
        &mut self,
        handle: &str,
        kind: ObservationKind,
        diff: bool,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        let tab = self
            .targets
            .get_mut(handle)
            .ok_or_else(|| failed("stale_tab: bind it again"))?;
        tab.refs.clear();
        tab.capture = None;
        tab.frame = None;
        tab.verify().await?;
        let result = tab.observe(kind, diff, cancellation).await;
        if result.is_err() {
            tab.socket = None;
            tab.refs.clear();
            tab.frame = None;
            tab.capture = None;
        }
        result
    }
    async fn action(
        &mut self,
        handle: &str,
        action: Action,
        cursor: &crate::cursor::Spec,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        let closing = matches!(&action, Action::Close);
        let moving_cursor = matches!(&action, Action::MoveCursor { .. });
        let tab = self
            .targets
            .get_mut(handle)
            .ok_or_else(|| failed("stale_tab: bind it again"))?;
        tab.verify().await?;
        let result = tab.action(action, cursor, cancellation).await;
        if !moving_cursor {
            tab.capture = None;
        }
        if result.is_err() {
            tab.socket = None;
            tab.refs.clear();
            tab.frame = None;
        }
        if closing && result.is_ok() {
            self.targets.remove(handle);
        }
        result
    }
    pub(crate) fn close(&mut self) {
        self.targets.clear();
    }
}
impl Tab {
    async fn wait_ready(&mut self) -> Result<(), ToolError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let tree = self.request("Page.getFrameTree", json!({})).await?;
            let frame = &tree["frameTree"]["frame"];
            let expected = match &self.navigation {
                None => true,
                Some(Navigation::Loader(loader)) => frame["loaderId"].as_str() == Some(loader),
                Some(Navigation::Reload(previous)) => frame["loaderId"]
                    .as_str()
                    .is_some_and(|loader| loader != previous),
                Some(Navigation::Url(url)) => frame["url"].as_str() == Some(url),
            };
            if expected {
                let ready = self
                    .request(
                        "Runtime.evaluate",
                        json!({"expression":"document.readyState","returnByValue":true}),
                    )
                    .await?;
                if matches!(
                    ready["result"]["value"].as_str(),
                    Some("interactive" | "complete")
                ) {
                    self.navigation = None;
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                self.navigation = None;
                return Err(failed(
                    "page has not reached an observable document; call getAXState again, without repeating navigation",
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    async fn verify(&mut self) -> Result<(), ToolError> {
        let (generation, tabs) = inventory(&self.provider).await?;
        if generation != self.generation || !tabs.iter().any(|tab| tab.id == self.info.id) {
            self.socket = None;
            self.refs.clear();
            self.capture = None;
            self.frame = None;
            return Err(failed(
                "stale_tab: browser restarted or tab closed; bind the current tab",
            ));
        }
        self.info = tabs
            .into_iter()
            .find(|tab| tab.id == self.info.id)
            .expect("verified tab identity");
        if self.socket.is_none() {
            self.refs.clear();
            self.capture = None;
            self.frame = None;
            let websocket = self
                .info
                .websocket
                .as_deref()
                .ok_or_else(|| failed("tab has no CDP websocket"))?;
            self.socket = Some(Cdp::connect(websocket, &self.provider).await?);
        }
        Ok(())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.socket
            .as_mut()
            .ok_or_else(|| failed("tab connection unavailable"))?
            .request(method, params)
            .await
    }
    async fn frame(&mut self) -> Result<String, ToolError> {
        let tree = self.request("Page.getFrameTree", json!({})).await?;
        let frame = &tree["frameTree"]["frame"];
        Ok(format!(
            "{}:{}",
            frame["id"]
                .as_str()
                .ok_or_else(|| failed("missing main frame"))?,
            frame["loaderId"]
                .as_str()
                .ok_or_else(|| failed("missing document generation"))?
        ))
    }
    async fn observe(
        &mut self,
        kind: ObservationKind,
        diff: bool,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        self.wait_ready().await?;
        crate::observation::settle(crate::observation::BROWSER_SETTLE, cancellation).await?;
        self.verify().await?;
        self.request(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":"maka-cua"}),
        )
        .await?;
        let frame = self.frame().await?;
        let mut value = json!({});
        if !matches!(kind, ObservationKind::Screenshot) {
            let tree = self
                .request("Accessibility.getFullAXTree", json!({"depth":30}))
                .await?;
            let nodes = tree["nodes"]
                .as_array()
                .ok_or_else(|| failed("browser returned no accessibility nodes"))?;
            let mut rows = vec![format!("Tab {}: {}", self.info.id, self.info.url)];
            let (tree, refs) = self.ax.render(&frame, nodes)?;
            rows.push(tree);
            self.refs = refs;
            if nodes.len() > 2000 {
                rows.push("Accessibility tree truncated at 2000 nodes.".into());
            }
            let full = rows.join("\n");
            value["state"] = json!(if diff {
                crate::session::display_diff(self.previous.as_deref(), &full)
            } else {
                full.clone()
            });
            self.previous = Some(full);
        } else {
            self.previous = None;
        }
        if !matches!(kind, ObservationKind::Ax) {
            let metrics = self.request("Page.getLayoutMetrics", json!({})).await?;
            let viewport = &metrics["cssVisualViewport"];
            let width = viewport["clientWidth"]
                .as_f64()
                .ok_or_else(|| failed("viewport dimensions unavailable"))?;
            let height = viewport["clientHeight"]
                .as_f64()
                .ok_or_else(|| failed("viewport dimensions unavailable"))?;
            let image = self
                .request(
                    "Page.captureScreenshot",
                    json!({"format":"png","captureBeyondViewport":false}),
                )
                .await?;
            let data = image["data"]
                .as_str()
                .ok_or_else(|| failed("browser screenshot unavailable"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(failed)?;
            if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
                return Err(failed("browser screenshot is not PNG"));
            }
            let image_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as f64;
            let image_height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as f64;
            if image_width == 0.0 || image_height == 0.0 {
                return Err(failed("empty browser screenshot"));
            }
            value["image"] = json!({"type":"image","mimeType":"image/png","data":data});
            self.capture = Some(Capture {
                width,
                height,
                image_width,
                image_height,
                x: viewport["pageX"].as_f64().unwrap_or(0.0),
                y: viewport["pageY"].as_f64().unwrap_or(0.0),
            });
        }
        if self.frame().await? != frame {
            return Err(failed("document changed during observation; observe again"));
        }
        self.frame = Some(frame);
        Ok(value)
    }
    async fn element(&mut self, index: u64) -> Result<String, ToolError> {
        let backend = *self
            .refs
            .get(&index)
            .ok_or_else(|| failed("stale_element: read getAXState first"))?;
        let resolved = self
            .request(
                "DOM.resolveNode",
                json!({"backendNodeId":backend,"objectGroup":"maka-cua"}),
            )
            .await?;
        resolved["object"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| failed("element is no longer attached"))
    }
    async fn call_element(
        &mut self,
        index: u64,
        function: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, ToolError> {
        let object = self.element(index).await?;
        // A child frame's bounding rect is local to that frame, while CDP
        // pointer input is relative to the main viewport. Refuse that semantic
        // route before focus/scroll/input instead of clicking another element.
        let function = format!(
            "function(...args) {{ const view=this.ownerDocument?.defaultView; if (!view || view !== view.top) throw Error('iframe semantic actions are unavailable; use the tab screenshot'); return ({function}).apply(this,args); }}"
        );
        let result = self.request("Runtime.callFunctionOn",json!({"objectId":object,"functionDeclaration":function,"arguments":arguments.into_iter().map(|value| json!({"value":value})).collect::<Vec<_>>(),"returnByValue":true})).await?;
        if let Some(error) = result.get("exceptionDetails") {
            return Err(failed(format!(
                "element operation refused: {}",
                error["exception"]["description"]
            )));
        }
        Ok(result["result"]["value"].clone())
    }
    async fn focus(&mut self, index: Option<u64>) -> Result<(), ToolError> {
        if let Some(index) = index {
            self.call_element(index,"function(){ if(!this.isConnected) throw Error('detached'); this.focus(); if(this.getRootNode().activeElement!==this) throw Error('focus not confirmed'); }",vec![]).await?;
        }
        Ok(())
    }
    async fn point(&mut self, point: [f64; 2]) -> Result<[f64; 2], ToolError> {
        let capture = self
            .capture
            .ok_or_else(|| failed("capture_required: getScreenshot before coordinate input"))?;
        let metrics = self.request("Page.getLayoutMetrics", json!({})).await?;
        if metrics["cssVisualViewport"]["clientWidth"].as_f64() != Some(capture.width)
            || metrics["cssVisualViewport"]["clientHeight"].as_f64() != Some(capture.height)
            || metrics["cssVisualViewport"]["pageX"].as_f64() != Some(capture.x)
            || metrics["cssVisualViewport"]["pageY"].as_f64() != Some(capture.y)
            || !point.iter().all(|v| v.is_finite() && *v >= 0.0)
            || point[0] >= capture.image_width
            || point[1] >= capture.image_height
        {
            return Err(failed(
                "stale_capture: viewport changed or point lies outside screenshot",
            ));
        }
        Ok([
            point[0] * capture.width / capture.image_width,
            point[1] * capture.height / capture.image_height,
        ])
    }
    async fn before_input(
        &mut self,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<(), ToolError> {
        check_input(cancellation)?;
        let frame = self.frame().await?;
        check_input(cancellation)?;
        if self.frame.as_ref() != Some(&frame) {
            self.refs.clear();
            self.capture = None;
            return Err(failed(
                "stale_document: observe after navigation before acting",
            ));
        }
        Ok(())
    }
    async fn action(
        &mut self,
        action: Action,
        cursor: &crate::cursor::Spec,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        if !matches!(
            &action,
            Action::Navigate { .. }
                | Action::Back
                | Action::Forward
                | Action::Reload
                | Action::Close
        ) {
            self.before_input(cancellation).await?;
        }
        if let Action::MoveCursor { target } = &action {
            let point = self.cursor_point(target).await?;
            return self
                .draw_cursor(cursor, Some(point), "idle", "update")
                .await;
        }
        if cursor.enabled
            && !matches!(
                action,
                Action::Click { .. } | Action::Scroll { .. } | Action::Drag { .. }
            )
            && let Some(target) = action.cursor_target()
            && let Ok(point) = self.cursor_point(&target).await
        {
            let _ = self
                .draw_cursor(
                    cursor,
                    Some(point),
                    action.cursor_action().as_str(),
                    "update",
                )
                .await;
        }
        check_input(cancellation)?;
        match action {
            Action::MoveCursor { .. } => unreachable!("handled before input dispatch"),
            Action::Navigate { url } => {
                validate_url(&url)?;
                self.refs.clear();
                self.capture = None;
                let result = self.request("Page.navigate", json!({"url":url})).await?;
                if let Some(error) = result.get("errorText") {
                    return Err(failed(format!("navigation failed: {error}")));
                }
                self.navigation = result["loaderId"]
                    .as_str()
                    .map(|loader| Navigation::Loader(loader.into()));
            }
            Action::Reload => {
                let tree = self.request("Page.getFrameTree", json!({})).await?;
                let previous = tree["frameTree"]["frame"]["loaderId"]
                    .as_str()
                    .ok_or_else(|| failed("document identity unavailable"))?
                    .to_owned();
                self.refs.clear();
                self.request("Page.reload", json!({})).await?;
                self.navigation = Some(Navigation::Reload(previous));
            }
            Action::Back | Action::Forward => {
                let history = self.request("Page.getNavigationHistory", json!({})).await?;
                let index = history["currentIndex"]
                    .as_i64()
                    .ok_or_else(|| failed("navigation history unavailable"))?
                    + if matches!(action, Action::Back) {
                        -1
                    } else {
                        1
                    };
                let entry = usize::try_from(index)
                    .ok()
                    .and_then(|index| history["entries"].get(index))
                    .ok_or_else(|| failed("no navigation history in that direction"))?;
                self.refs.clear();
                self.navigation = entry["url"].as_str().map(|url| Navigation::Url(url.into()));
                self.request(
                    "Page.navigateToHistoryEntry",
                    json!({"entryId":entry["id"]}),
                )
                .await?;
            }
            Action::Close => {
                self.refs.clear();
                self.request("Page.close", json!({})).await?;
            }
            Action::TypeText { index, text } => {
                self.focus(index).await?;
                self.before_input(cancellation).await?;
                self.request("Input.insertText", json!({"text":text}))
                    .await?;
            }
            Action::Paste {
                index,
                text,
                options,
            } => {
                if matches!(options.format, Some(PasteFormat::Html)) {
                    let index = index.ok_or_else(|| {
                        failed("HTML paste requires an observed editable element")
                    })?;
                    self.call_element(index,"function(html){ if(!this.isConnected || !this.isContentEditable) throw Error('HTML paste requires contenteditable'); this.focus(); if(!document.execCommand('insertHTML',false,html)) throw Error('HTML insertion not supported'); }",vec![json!(text)]).await?;
                } else {
                    self.focus(index).await?;
                    self.before_input(cancellation).await?;
                    self.request("Input.insertText", json!({"text":text}))
                        .await?;
                }
            }
            Action::SetValue { index, value } => {
                self.call_element(
                    index,
                    include_str!("browser/set-value.js"),
                    vec![json!(value)],
                )
                .await?;
            }
            Action::SelectText {
                index,
                text,
                options,
            } => {
                let selection = match options.selection_type.unwrap_or(SelectionType::Text) {
                    SelectionType::Text => "text",
                    SelectionType::CursorBefore => "cursor_before",
                    SelectionType::CursorAfter => "cursor_after",
                };
                self.call_element(
                    index,
                    include_str!("browser/select-text.js"),
                    vec![
                        json!(text),
                        json!(options.prefix),
                        json!(options.suffix),
                        json!(selection),
                    ],
                )
                .await?;
            }
            Action::Click { target, options } => {
                let count = options.click_count.unwrap_or(1);
                if !(1..=3).contains(&count) {
                    return Err(failed("clickCount must be 1 to 3"));
                }
                let button = options.mouse_button.unwrap_or(MouseButton::Left).name();
                let point = match target {
                    Position::Point(point) => self.point(point).await?,
                    Position::Element(index) => {
                        let point=self.call_element(index,"function(){ if(!this.isConnected || this.disabled) throw Error('element unavailable'); this.scrollIntoView({block:'center',inline:'center'}); const r=this.getBoundingClientRect(); if(r.width<=0||r.height<=0) throw Error('element has no visible area'); const x=r.x+r.width/2,y=r.y+r.height/2; const hit=this.getRootNode().elementFromPoint(x,y); if(hit!==this&&!this.contains(hit)) throw Error('element is occluded'); return [x,y]; }",vec![]).await?;
                        serde_json::from_value(point).map_err(failed)?
                    }
                };
                if cursor.enabled {
                    let _ = self
                        .draw_cursor(cursor, Some(point), "click", "update")
                        .await;
                }
                self.before_input(cancellation).await?;
                let pressed=self.request("Input.dispatchMouseEvent",json!({"type":"mousePressed","x":point[0],"y":point[1],"button":button,"clickCount":count})).await;
                self.request("Input.dispatchMouseEvent",json!({"type":"mouseReleased","x":point[0],"y":point[1],"button":button,"clickCount":count})).await.map_err(|error|ToolError::CleanupUnconfirmed(format!("mouse release was not acknowledged: {error}")))?;
                pressed?;
            }
            Action::PressKey { index, key } => {
                let (key, code, modifiers) = key_event(&key)?;
                self.focus(index).await?;
                self.before_input(cancellation).await?;
                let mut event = json!({"type":"keyDown","key":key,"windowsVirtualKeyCode":code,"modifiers":modifiers});
                if modifiers & 7 == 0 {
                    if key.len() == 1 {
                        event["text"] = json!(key);
                    } else if key == "Enter" {
                        event["text"] = json!("\r");
                    }
                }
                let pressed = self.request("Input.dispatchKeyEvent", event).await;
                self.request("Input.dispatchKeyEvent",json!({"type":"keyUp","key":key,"windowsVirtualKeyCode":code,"modifiers":modifiers})).await.map_err(|error|ToolError::CleanupUnconfirmed(format!("key release was not acknowledged: {error}")))?;
                pressed?;
            }
            Action::Scroll {
                target,
                direction,
                distance,
            } => {
                let pixels = match distance.unwrap_or(Distance::Pages(1)) {
                    Distance::Pages(pages) => {
                        let layout = self.request("Page.getLayoutMetrics", json!({})).await?;
                        let dimension = if matches!(direction, Direction::Left | Direction::Right) {
                            "clientWidth"
                        } else {
                            "clientHeight"
                        };
                        let extent = layout["cssVisualViewport"][dimension]
                            .as_f64()
                            .ok_or_else(|| failed("viewport dimensions unavailable"))?;
                        extent * pages as f64
                    }
                    Distance::Pixels { pixels } => pixels as f64,
                };
                if !(1.0..=50000.0).contains(&pixels) {
                    return Err(failed("scroll distance out of range"));
                }
                let (dx, dy) = match direction {
                    Direction::Up => (0.0, -pixels),
                    Direction::Down => (0.0, pixels),
                    Direction::Left => (-pixels, 0.0),
                    Direction::Right => (pixels, 0.0),
                };
                let point=match target {Position::Point(point)=>self.point(point).await?,Position::Element(index)=>serde_json::from_value(self.call_element(index,"function(){const r=this.getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2];}",vec![]).await?).map_err(failed)?};
                if cursor.enabled {
                    let _ = self
                        .draw_cursor(cursor, Some(point), "scroll", "update")
                        .await;
                }
                self.before_input(cancellation).await?;
                self.request(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseWheel","x":point[0],"y":point[1],"deltaX":dx,"deltaY":dy}),
                )
                .await?;
            }
            Action::Drag { from, to } => {
                let from = self.point(from).await?;
                let to = self.point(to).await?;
                if cursor.enabled {
                    let _ = self.draw_cursor(cursor, Some(from), "drag", "update").await;
                }
                self.before_input(cancellation).await?;
                let pressed=self.request("Input.dispatchMouseEvent",json!({"type":"mousePressed","x":from[0],"y":from[1],"button":"left","clickCount":1})).await;
                let moved = if pressed.is_ok() {
                    if cursor.enabled {
                        let _ = self.draw_cursor(cursor, Some(to), "drag", "update").await;
                    }
                    if let Err(error) = self.before_input(cancellation).await {
                        Err(error)
                    } else {
                        self.request("Input.dispatchMouseEvent",json!({"type":"mouseMoved","x":to[0],"y":to[1],"button":"left","buttons":1})).await
                    }
                } else {
                    pressed
                };
                let released=self.request("Input.dispatchMouseEvent",json!({"type":"mouseReleased","x":to[0],"y":to[1],"button":"left","clickCount":1})).await;
                released.map_err(|error| {
                    ToolError::CleanupUnconfirmed(format!(
                        "mouse release was not acknowledged: {error}"
                    ))
                })?;
                moved?;
            }
            Action::Secondary { .. } => {
                return Err(failed(
                    "unsupported: CDP accessibility does not expose secondary action names",
                ));
            }
        }
        if self.navigation.is_some() {
            self.wait_ready().await?;
        }
        Ok(Value::Null)
    }
}
fn validate_url(url: &str) -> Result<(), ToolError> {
    if url == "about:blank" {
        return Ok(());
    }
    let parsed = reqwest::Url::parse(url).map_err(failed)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(failed(
            "browser navigation supports HTTP(S) and about:blank",
        ));
    }
    Ok(())
}
fn key_event(key: &str) -> Result<(String, u32, u32), ToolError> {
    let mut parts: Vec<_> = key.split('+').collect();
    let key = parts.pop().unwrap_or_default();
    let mut modifiers = 0;
    for part in parts {
        modifiers |= match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => 1,
            "ctrl" | "control" => 2,
            "super" | "meta" | "cmd" | "command" => 4,
            "shift" => 8,
            _ => return Err(failed("unsupported key modifier")),
        };
    }
    let (name, code) = match key {
        "Return" | "Enter" => ("Enter", 13),
        "Tab" => ("Tab", 9),
        "Escape" | "Esc" => ("Escape", 27),
        "BackSpace" | "Backspace" => ("Backspace", 8),
        "Delete" => ("Delete", 46),
        "Up" | "ArrowUp" => ("ArrowUp", 38),
        "Down" | "ArrowDown" => ("ArrowDown", 40),
        "Left" | "ArrowLeft" => ("ArrowLeft", 37),
        "Right" | "ArrowRight" => ("ArrowRight", 39),
        "Home" => ("Home", 36),
        "End" => ("End", 35),
        "Page_Up" | "PageUp" => ("PageUp", 33),
        "Page_Down" | "PageDown" => ("PageDown", 34),
        key if key.len() == 1 && key.is_ascii() => {
            (key, key.as_bytes()[0].to_ascii_uppercase() as u32)
        }
        _ => return Err(failed("unsupported browser key")),
    };
    Ok((name.into(), code, modifiers))
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
fn unknown(error: impl std::fmt::Display) -> ToolError {
    ToolError::OutcomeUnknown(error.to_string())
}

fn check_input(cancellation: &tokio_util::sync::CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(failed("Computer Use cancelled before input dispatch"))
    } else {
        Ok(())
    }
}
