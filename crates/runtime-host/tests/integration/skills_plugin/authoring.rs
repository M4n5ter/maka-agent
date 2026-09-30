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

use super::{Host, HostFixture, Peer, Provider, client, configure, converged, disabled};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bundled_authoring_installs_complete_resources_and_runs_its_starter() {
    tokio::time::timeout(Duration::from_secs(45), scenario())
        .await
        .unwrap();
}

async fn scenario() {
    let fixture = HostFixture::new("maka-authoring-");
    let (provider, mut requests) = Provider::controlled().await;
    let model = configure(&fixture, &provider.base_url).await;
    let host = Host::open(fixture.owner()).await.unwrap();
    let mut peer = Peer::new(host.clone(), "authoring-client").await;
    converged(&mut peer).await;
    let workspace = json!(fixture.workspace);
    let sources = client::workspace(
        &mut peer,
        &workspace,
        json!({"kind":"catalog", "view":"bundled"}),
    )
    .await;
    for id in ["maka-cua", "maka-plugin-authoring"] {
        let item = sources["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap();
        assert_eq!(item["installed"], true, "{item}");
    }
    let location = client::workspace(&mut peer, &workspace, json!({
        "kind":"resolve_path", "ref":"workspace:legacy:maka-plugin-authoring", "target":"directory"
    })).await;
    let directory = std::path::PathBuf::from(location["path"].as_str().unwrap());
    let sdk = directory.join("references/sdk/host.ts");
    assert_eq!(
        std::fs::read(&sdk).unwrap(),
        include_bytes!("../../../../../packages/plugin-sdk/src/host.ts")
    );
    let installed = peer
        .rpc(
            "plugin.package.install",
            json!({"sourcePath":directory.join("assets/starter")}),
        )
        .await;
    assert_eq!(installed["ok"], true, "{installed}");
    converged(&mut peer).await;
    let created = peer.rpc("session.create", json!({
        "sessionId":"authoring", "workspace":{"kind":"host_path","path":fixture.workspace},
        "modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model},
        "sandboxMode":"workspace-write", "mode":"bot"
    })).await;
    assert_eq!(created["ok"], true, "{created}");
    let started = peer.rpc("turn.start", json!({"sessionId":"authoring", "turnId":"read-skill", "content":{"text":"Inspect the plugin authoring resources and run the starter."}, "maxSteps":8})).await;
    assert_eq!(started["ok"], true, "{started}");
    let request = requests.recv().await.unwrap();
    assert!(request.body.to_string().contains("maka-plugin-authoring"));
    std::fs::write(&sdk, "Changed resource contract").unwrap();
    request
        .reply
        .send(call(
            "Skill",
            json!({"name":"maka-plugin-authoring","resource":{"path":"references/sdk/host.ts"}}),
        ))
        .unwrap();
    let request = requests.recv().await.unwrap();
    let result: Value = serde_json::from_str(last_tool(&request.body)).unwrap();
    assert_eq!(result["status"], "resource", "{result}");
    assert!(
        result["page"]["content"]
            .as_str()
            .unwrap()
            .contains("HOST_SDK_VERSION = 3")
    );
    let next = result["page"]["next"].clone();
    assert_eq!(next["name"], "workspace:legacy:maka-plugin-authoring");
    request.reply.send(call("Skill", next)).unwrap();
    let request = requests.recv().await.unwrap();
    assert!(last_tool(&request.body).contains("changed"));
    request
        .reply
        .send(call(
            "Skill",
            json!({"name":"maka-plugin-authoring","resource":{"path":"references/sdk/host.ts"}}),
        ))
        .unwrap();
    let request = requests.recv().await.unwrap();
    let result: Value = serde_json::from_str(last_tool(&request.body)).unwrap();
    assert_eq!(result["page"]["content"], "Changed resource contract");
    request
        .reply
        .send(call("tool_search", json!({"query":"ExampleEcho"})))
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert!(
        request.body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "ExampleEcho")
    );
    request
        .reply
        .send(call("ExampleEcho", json!({"text":"skill-starter-proof"})))
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert!(last_tool(&request.body).contains("skill-starter-proof"));
    request
        .reply
        .send(json!({"index":0,"delta":{"content":"done"},"finish_reason":"stop"}))
        .unwrap();
    loop {
        let state = peer
            .rpc(
                "turn.query",
                json!({"sessionId":"authoring","turnId":"read-skill"}),
            )
            .await;
        match state["result"]["status"].as_str() {
            Some("completed") => break,
            Some("failed" | "cancelled") => panic!("{state}"),
            _ => tokio::task::yield_now().await,
        }
    }
    // Bundled preferences survive reactivation while shipped resources are restored.
    let basis = client::workspace(
        &mut peer,
        &workspace,
        json!({"kind":"catalog","view":"governance"}),
    )
    .await;
    let result = client::workspace(&mut peer, &workspace, json!({"kind":"mutate","expectedRevision":basis["revision"],"mutation":{"kind":"set_preferences","ref":"workspace:legacy:maka-plugin-authoring","enabled":false,"pinned":true}})).await;
    assert_eq!(result["kind"], "committed", "{result}");
    let basis = client::workspace(
        &mut peer,
        &workspace,
        json!({"kind":"catalog","view":"governance"}),
    )
    .await;
    let removal = client::workspace(&mut peer, &workspace, json!({"kind":"mutate","expectedRevision":basis["revision"],"mutation":{"kind":"delete","ref":"workspace:legacy:maka-cua"}})).await;
    assert_eq!(removal["reason"], "blocked_scope", "{removal}");
    disabled(&mut peer, true).await;
    disabled(&mut peer, false).await;
    let installed = client::workspace(
        &mut peer,
        &workspace,
        json!({"kind":"catalog","view":"governance"}),
    )
    .await;
    let items = installed["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|item| item["ref"] == "workspace:legacy:maka-cua")
    );
    let authoring = items
        .iter()
        .find(|item| item["ref"] == "workspace:legacy:maka-plugin-authoring")
        .unwrap();
    assert_eq!(authoring["enabled"], false);
    assert_eq!(authoring["pinned"], true);
    assert_eq!(
        std::fs::read(&sdk).unwrap(),
        include_bytes!("../../../../../packages/plugin-sdk/src/host.ts")
    );
    peer.close().await;
    drop(host);
}
fn call(name: &str, input: Value) -> Value {
    json!({"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":name,"arguments":input.to_string()}}]},"finish_reason":"tool_calls"})
}
fn last_tool(request: &Value) -> &str {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
}
