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

use super::{rpc, success};
use crate::support::peer::Peer;
use serde_json::{Value, json};

pub(super) async fn verify(
    peer: &mut Peer,
    client: &Value,
    document: &Value,
    workspace: &std::path::Path,
) {
    let binding = json!({"client":client,"method":"controls","sessionId":null});
    let target = rpc(peer, json!({"kind":"bind","binding":binding})).await["target"].clone();
    let call = json!({"kind":"call","binding":binding,"target":target,"document":document});
    async fn invoke(peer: &mut Peer, call: &Value, input: Value) -> Value {
        let mut request = call.clone();
        request["input"] = input;
        rpc(peer, request).await["value"].clone()
    }
    let original = invoke(peer, &call, json!({"kind":"preferences"})).await;
    let update = json!({"expectedRevision":original["revision"],"mutation":{"kind":"personalization","value":{"displayName":"M4n5ter","assistantTone":"Brief"}}});
    assert_eq!(
        invoke(
            peer,
            &call,
            json!({"kind":"update_preferences","input":update})
        )
        .await["kind"],
        "committed"
    );
    assert_eq!(
        invoke(
            peer,
            &call,
            json!({"kind":"update_preferences","input":update})
        )
        .await["kind"],
        "revision_conflict"
    );
    let preferences = invoke(peer, &call, json!({"kind":"preferences"})).await;
    assert_eq!(preferences["personalization"]["displayName"], "M4n5ter");
    assert_eq!(preferences["privacy"], original["privacy"]);
    let input = json!({"sessionId":"controlled-session","workspace":{"kind":"host_path","path":workspace},"executorId":"example","settings":{"model":"custom-model"},"name":"Original"});
    assert!(
        invoke(peer, &call, json!({"kind":"creation","input":input}))
            .await
            .is_null()
    );
    let created = invoke(peer, &call, json!({"kind":"create","input":input})).await;
    assert_eq!(created["executorId"], "example");
    assert_eq!(created["settings"]["model"], "custom-model");
    assert_eq!(
        invoke(peer, &call, json!({"kind":"creation","input":input})).await,
        created
    );
    let mut changed = input.clone();
    changed["name"] = json!("Different");
    let mut mismatch = call.clone();
    mismatch["input"] = json!({"kind":"creation","input":changed});
    assert_eq!(
        peer.rpc("plugin.remote", mismatch).await["ok"],
        false,
        "receipt binds the original creation fingerprint"
    );
    let mut bound = binding;
    bound["sessionId"] = json!("controlled-session");
    let target = rpc(peer, json!({"kind":"bind","binding":bound})).await["target"].clone();
    let configured = json!({"kind":"call","binding":bound,"target":target,"document":document});
    let patch = json!({"expectedRevision":created["revision"],"executorId":"example","settings":{"model":"next-model"}});
    assert_eq!(
        invoke(peer, &configured, json!({"kind":"configure","input":patch})).await["kind"],
        "committed"
    );
    assert_eq!(
        invoke(peer, &configured, json!({"kind":"configure","input":patch})).await["kind"],
        "revision_conflict"
    );
    let session = invoke(
        peer,
        &configured,
        json!({"kind":"session","input":"controlled-session"}),
    )
    .await;
    assert_eq!(session["settings"]["model"], "next-model");
    let canonical = success(
        peer.rpc(
            "session.catalog.query",
            json!({"kind":"get","sessionId":"controlled-session"}),
        )
        .await,
    );
    assert_eq!(
        canonical["session"]["executorSettings"]["model"],
        "next-model"
    );
}

async fn form_call(peer: &mut Peer, document: &Value, input: Value) -> Value {
    let directory = success(
        peer.rpc(
            "plugin.platform.query",
            json!({"view":"terminal_views","rootId":"profile"}),
        )
        .await,
    );
    let row = directory["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["packageId"] == "example.remote" && row["descriptor"]["launch"] == true)
        .unwrap();
    let binding = json!({"packageId":row["packageId"],"method":row["method"],"sessionId":null});
    rpc(peer, json!({"kind":"call","binding":binding,"target":row["target"],"document":document,"input":input})).await["value"].clone()
}
pub(super) async fn create_with_form(
    peer: &mut Peer,
    document: &Value,
    workspace: &std::path::Path,
) -> Value {
    let route = json!({"workspace":{"kind":"host_path","path":workspace}});
    let read = form_call(
        peer,
        document,
        json!({"kind":"read","route":route,"locale":"en"}),
    )
    .await;
    let view: maka_plugins::terminal_ui::view::View =
        serde_json::from_value(read["view"].clone()).unwrap();
    view.validate().unwrap();
    let operation = view.revision;
    let recovery = json!({"operation":operation});
    assert_eq!(view.actions[0].recovery, Some(recovery.clone()));
    let input = json!({"kind":"submit","route":route,"revision":operation,"action":"save","fields":{"model":"from-plugin-model","name":"From plugin","path":workspace,"thinking":"default"},"grant":null,"locale":"en"});
    let applied = form_call(peer, document, input).await;
    assert_eq!(applied["kind"], "applied");
    assert_eq!(applied["route"]["session"], operation);
    recovery
}
pub(super) async fn recover_form(peer: &mut Peer, document: &Value, route: &Value) {
    let receipt = form_call(
        peer,
        document,
        json!({"kind":"recover","route":route,"locale":"en"}),
    )
    .await;
    assert_eq!(receipt["kind"], "applied");
    assert_eq!(receipt["route"]["session"], route["operation"]);
    let original = success(
        peer.rpc(
            "session.catalog.query",
            json!({"kind":"get","sessionId":route["operation"]}),
        )
        .await,
    );
    assert_eq!(original["session"]["name"], "From plugin");
    assert_eq!(
        original["session"]["executorSettings"]["model"],
        "from-plugin-model"
    );
}
