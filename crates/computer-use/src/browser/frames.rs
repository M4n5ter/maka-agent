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

use super::*;
use cua_driver_core::browser::cdp_ws::CdpEvent;
use tokio::sync::mpsc;

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
pub(super) struct Frame {
    pub id: String,
    pub loader: String,
    pub session: Option<String>,
    pub parent: Option<String>,
}

#[derive(Clone)]
struct Child {
    session: String,
    parent: Option<String>,
}

#[derive(Default)]
pub(super) struct Frames {
    events: Option<mpsc::UnboundedReceiver<CdpEvent>>,
    children: HashMap<String, Child>,
}

impl Tab {
    pub(super) async fn frame_request(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, ToolError> {
        self.socket
            .as_ref()
            .ok_or_else(|| failed("tab connection unavailable"))?
            .request_in(session, method, params)
            .await
    }

    pub(super) async fn frames(&mut self) -> Result<Vec<Frame>, ToolError> {
        if self.frames.events.is_none() {
            let connection = &self.socket.as_ref().unwrap().connection;
            self.frames.events = Some(connection.subscribe());
            self.frame_request(
                None,
                "Target.setAutoAttach",
                json!({
                    "autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true
                }),
            )
            .await?;
        }
        // Only exact descendants announced on this bound page socket are
        // admitted. Popups, workers and unrelated target inventories never do.
        for _ in 0..16 {
            let mut attach = Vec::new();
            while let Ok(event) = self.frames.events.as_mut().unwrap().try_recv() {
                let parent_known = event.session_id.is_none()
                    || self
                        .frames
                        .children
                        .values()
                        .any(|child| Some(&child.session) == event.session_id.as_ref());
                if !parent_known {
                    continue;
                }
                if event.method == "Target.detachedFromTarget" {
                    if let Some(session) = event.params["sessionId"].as_str() {
                        self.frames
                            .children
                            .retain(|_, child| child.session != session);
                        loop {
                            let live: std::collections::HashSet<_> = self
                                .frames
                                .children
                                .values()
                                .map(|child| child.session.clone())
                                .collect();
                            let before = self.frames.children.len();
                            self.frames.children.retain(|_, child| {
                                child
                                    .parent
                                    .as_ref()
                                    .is_none_or(|parent| live.contains(parent))
                            });
                            if before == self.frames.children.len() {
                                break;
                            }
                        }
                    }
                } else if event.method == "Target.attachedToTarget"
                    && event.params["targetInfo"]["type"] == "iframe"
                {
                    let (Some(target), Some(session)) = (
                        event.params["targetInfo"]["targetId"].as_str(),
                        event.params["sessionId"].as_str(),
                    ) else {
                        continue;
                    };
                    if self.frames.children.len() >= 32 {
                        return Err(failed("iframe session limit reached"));
                    }
                    if self
                        .frames
                        .children
                        .get(target)
                        .is_none_or(|child| child.session != session)
                    {
                        self.frames.children.insert(
                            target.to_owned(),
                            Child {
                                session: session.to_owned(),
                                parent: event.session_id.clone(),
                            },
                        );
                        attach.push(session.to_owned());
                    }
                }
            }
            if attach.is_empty() {
                break;
            }
            for session in attach {
                if !self
                    .frames
                    .children
                    .values()
                    .any(|child| child.session == session)
                {
                    continue;
                }
                self.frame_request(
                    Some(&session),
                    "Target.setAutoAttach",
                    json!({
                        "autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true
                    }),
                )
                .await?;
            }
        }
        let main = self
            .frame_request(None, "Page.getFrameTree", json!({}))
            .await?;
        let mut frames = Vec::new();
        collect_frames(&main["frameTree"], None, &mut frames)?;
        let mut children: Vec<_> = self.frames.children.values().cloned().collect();
        children.sort_by(|a, b| a.session.cmp(&b.session));
        for child in children {
            let tree = self
                .frame_request(Some(&child.session), "Page.getFrameTree", json!({}))
                .await?;
            let id = tree["frameTree"]["frame"]["id"]
                .as_str()
                .ok_or_else(|| failed("iframe identity unavailable"))?;
            // Prove an owner in the exact parent renderer, rather than trusting
            // a claimed target id or a renderer-controlled URL.
            self.frame_request(
                child.parent.as_deref(),
                "DOM.getFrameOwner",
                json!({"frameId":id}),
            )
            .await?;
            let mut local = Vec::new();
            collect_frames(&tree["frameTree"], Some(&child.session), &mut local)?;
            for frame in local {
                frames.retain(|old| old.id != frame.id);
                frames.push(frame);
            }
        }
        if frames.len() > 64 {
            return Err(failed("frame observation limit reached"));
        }
        Ok(frames)
    }

    pub(super) async fn verify_element(&mut self, index: u64) -> Result<ElementRef, ToolError> {
        let element = self
            .refs
            .get(&index)
            .cloned()
            .ok_or_else(|| failed("stale_element: read getAXState first"))?;
        let frame = element
            .frame
            .as_ref()
            .ok_or_else(|| failed("element frame was not proven"))?;
        if !self.frames().await?.contains(frame) {
            self.refs.clear();
            return Err(failed(
                "stale_element: frame navigated or detached; observe again",
            ));
        }
        Ok(element)
    }

    pub(super) async fn element_point(
        &mut self,
        index: u64,
        scroll: bool,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<[f64; 2], ToolError> {
        let element = self.verify_element(index).await?;
        let value=self.call_element(index,"function(scroll){if(!this.isConnected||this.disabled)throw Error('element unavailable');if(scroll)this.scrollIntoView({block:'center',inline:'center'});const r=this.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;const hit=this.getRootNode().elementFromPoint(x,y);if(r.width<=0||r.height<=0||!(hit===this||this.contains(hit)))throw Error('element occluded');return {point:[x,y],width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight};}",vec![json!(scroll)],cancellation).await?;
        let mut point: [f64; 2] = serde_json::from_value(value["point"].clone()).map_err(failed)?;
        let mut width = value["width"]
            .as_f64()
            .ok_or_else(|| failed("frame width unavailable"))?;
        let mut height = value["height"]
            .as_f64()
            .ok_or_else(|| failed("frame height unavailable"))?;
        let frames = self.frames().await?;
        let mut frame = element
            .frame
            .clone()
            .ok_or_else(|| failed("element frame was not proven"))?;
        for _ in 0..64 {
            let Some(parent) = frame.parent.as_ref() else {
                return Ok(point);
            };
            let parent = frames
                .iter()
                .find(|f| &f.id == parent)
                .ok_or_else(|| failed("frame parent unavailable"))?
                .clone();
            let owner = self
                .frame_request(
                    parent.session.as_deref(),
                    "DOM.getFrameOwner",
                    json!({"frameId":frame.id}),
                )
                .await?;
            let backend = owner["backendNodeId"]
                .as_u64()
                .ok_or_else(|| failed("frame owner unavailable"))?;
            let resolved = self
                .frame_request(
                    parent.session.as_deref(),
                    "DOM.resolveNode",
                    json!({"backendNodeId":backend,"objectGroup":"maka-cua"}),
                )
                .await?;
            let object = resolved["object"]["objectId"]
                .as_str()
                .ok_or_else(|| failed("iframe owner detached"))?;
            let checked=self.frame_request(parent.session.as_deref(),"Runtime.callFunctionOn",json!({
                "objectId":object,"functionDeclaration":"function(x,y,width,height){if(!this.isConnected)throw Error('iframe owner detached');for(let node=this;node;node=node.assignedSlot??node.parentElement??node.getRootNode().host){const style=getComputedStyle(node);if(style.transform!=='none'||style.rotate!=='none'||style.scale!=='none'||style.translate!=='none')throw Error('transformed iframe input unavailable; use a tab screenshot');}const r=this.getBoundingClientRect(),sx=r.width/this.offsetWidth,sy=r.height/this.offsetHeight;const px=r.x+this.clientLeft*sx+x/width*this.clientWidth*sx,py=r.y+this.clientTop*sy+y/height*this.clientHeight*sy;const hit=this.getRootNode().elementFromPoint(px,py);if(hit!==this)throw Error('iframe owner occluded');return {point:[px,py],width:this.ownerDocument.defaultView.innerWidth,height:this.ownerDocument.defaultView.innerHeight};}",
                "arguments":[{"value":point[0]},{"value":point[1]},{"value":width},{"value":height}],"returnByValue":true
            })).await?;
            if checked.get("exceptionDetails").is_some() {
                return Err(failed(
                    "iframe owner occluded, transformed or detached; use a fresh tab screenshot",
                ));
            }
            let checked = &checked["result"]["value"];
            point = serde_json::from_value(checked["point"].clone()).map_err(failed)?;
            width = checked["width"]
                .as_f64()
                .ok_or_else(|| failed("parent viewport unavailable"))?;
            height = checked["height"]
                .as_f64()
                .ok_or_else(|| failed("parent viewport unavailable"))?;
            if !point.iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0 {
                return Err(failed("iframe geometry unavailable"));
            }
            frame = parent;
        }
        Err(failed("iframe ancestry limit reached"))
    }
}

fn collect_frames(
    tree: &Value,
    session: Option<&str>,
    frames: &mut Vec<Frame>,
) -> Result<(), ToolError> {
    let raw = &tree["frame"];
    if let (Some(id), Some(loader)) = (raw["id"].as_str(), raw["loaderId"].as_str()) {
        frames.push(Frame {
            id: id.into(),
            loader: loader.into(),
            session: session.map(str::to_owned),
            parent: raw["parentId"].as_str().map(str::to_owned),
        });
    }
    if let Some(children) = tree["childFrames"].as_array() {
        for child in children {
            collect_frames(child, session, frames)?;
        }
    }
    Ok(())
}
