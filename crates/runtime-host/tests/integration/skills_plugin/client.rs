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

use super::{Peer, json};

pub(crate) async fn authorization(
    peer: &mut Peer,
    command: serde_json::Value,
) -> serde_json::Value {
    let binding = json!({"packageId":"maka.skills","method":"request"});
    let bound = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(bound["ok"], true, "{bound}");
    let result = peer
        .rpc(
            "plugin.authorization",
            json!({"binding":binding,"target":bound["result"]["target"],"command":command}),
        )
        .await;
    assert_eq!(result["ok"], true, "{result}");
    result["result"].clone()
}

pub(crate) async fn request(
    peer: &mut Peer,
    method: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let binding = json!({"packageId":"maka.skills","method":method});
    let bound = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(bound["ok"], true, "{bound}");
    let opened = peer
        .rpc("plugin.remote", json!({"kind":"open_document"}))
        .await;
    assert_eq!(opened["ok"], true, "{opened}");
    let document = &opened["result"]["document"];
    let outcome = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"call","document":document,
        "binding":binding,"target":bound["result"]["target"],"input":input}),
        )
        .await;
    let closed = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"close_document","document":document}),
        )
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(outcome["ok"], true, "{outcome}");
    outcome["result"]["value"].clone()
}

pub(crate) async fn workspace(
    peer: &mut Peer,
    path: &serde_json::Value,
    input: serde_json::Value,
) -> serde_json::Value {
    request(
        peer,
        "path-request",
        json!({"path":path,"sandboxMode":"workspace-write",
        "collaborationMode":"agent","request":input}),
    )
    .await
}
