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
use crate::cursor::Spec;

impl Browsers {
    pub(crate) async fn configure_cursor(&mut self, cursor: &Spec) {
        for tab in self.targets.values_mut() {
            if tab.verify().await.is_ok() {
                let _ = tab.draw_cursor(cursor, None, "idle", "update").await;
            }
        }
    }
    pub(crate) async fn cursor_state(&mut self, cursor: &Spec) -> Vec<Value> {
        let mut states = Vec::new();
        for tab in self.targets.values_mut() {
            if tab.verify().await.is_ok() {
                match tab.draw_cursor(cursor, None, "idle", "state").await {
                    Ok(state) => states.push(json!({"tab":tab.info.id,"state":state})),
                    Err(error) => states.push(json!({"tab":tab.info.id,"error":error.to_string()})),
                }
            }
        }
        states
    }
    pub(crate) async fn remove_cursors(&mut self, cursor: &Spec) {
        for tab in self.targets.values_mut() {
            if tab.verify().await.is_ok() {
                let _ = tab.draw_cursor(cursor, None, "idle", "remove").await;
            }
        }
    }
}
impl Tab {
    pub(super) async fn draw_cursor(
        &mut self,
        cursor: &Spec,
        point: Option<[f64; 2]>,
        action: &str,
        operation: &str,
    ) -> Result<Value, ToolError> {
        let tree = self.request("Page.getFrameTree", json!({})).await?;
        let frame = &tree["frameTree"]["frame"];
        let id = frame["id"]
            .as_str()
            .ok_or_else(|| failed("cursor frame unavailable"))?;
        let identity = format!("{}:{}", id, frame["loaderId"].as_str().unwrap_or_default());
        if operation == "update" && self.frame.as_ref() != Some(&identity) {
            return Err(failed("stale_document: observe before showing its cursor"));
        }
        let context = match &self.cursor_world {
            Some((previous, context)) if *previous == identity => *context,
            _ => {
                let world=self.request("Page.createIsolatedWorld",json!({"frameId":id,"worldName":"maka-cua-cursor","grantUniveralAccess":false})).await?;
                let context = world["executionContextId"]
                    .as_u64()
                    .ok_or_else(|| failed("cursor isolated world unavailable"))?;
                self.cursor_world = Some((identity, context));
                context
            }
        };
        let input = json!({"cursor":cursor,"point":point,"action":action,"operation":operation,"svg":crate::cursor::theme::svg(cursor.color)});
        let value = self
            .request(
                "Runtime.evaluate",
                json!({
                    "expression":format!("({})({input})",include_str!("cursor.js")),
                    "contextId":context,"returnByValue":true
                }),
            )
            .await?;
        if value.get("exceptionDetails").is_some() {
            self.cursor_world = None;
            return Err(failed("browser cursor could not be rendered"));
        }
        let state: crate::cursor::RenderState =
            serde_json::from_value(value["result"]["value"].clone()).map_err(failed)?;
        Ok(json!(state))
    }
    pub(super) async fn cursor_point(&mut self, target: &Position) -> Result<[f64; 2], ToolError> {
        match target {
            Position::Point(point) => self.point(*point).await,
            Position::Element(index) => {
                let value=self.call_element(*index,"function(){const r=this.getBoundingClientRect(); if(!this.isConnected||r.width<=0||r.height<=0)throw Error('cursor target unavailable'); return [r.x+r.width/2,r.y+r.height/2];}",vec![]).await?;
                serde_json::from_value(value).map_err(failed)
            }
        }
    }
}
