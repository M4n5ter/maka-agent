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

const PACKAGE: &str = "example.javascript-context";
const SESSION: &str = "javascript-context";

async fn remote(peer: &mut Peer, method: &str, input: Value, session: Option<&str>) -> Value {
    let binding = json!({"packageId":PACKAGE,"method":method,"sessionId":session});
    let bound = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(bound["ok"], true, "{bound}");
    let opened = peer
        .rpc("plugin.remote", json!({"kind":"open_document"}))
        .await;
    let document = &opened["result"]["document"];
    let result = peer.rpc("plugin.remote",json!({"kind":"call","binding":binding,"target":bound["result"]["target"],"document":document,"input":input})).await;
    let closed = peer
        .rpc(
            "plugin.remote",
            json!({"kind":"close_document","document":document}),
        )
        .await;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(result["ok"], true, "{result}");
    result["result"]["value"].clone()
}
async fn response(peer: &mut Peer) -> Value {
    loop {
        let value = peer.frame().await;
        if value.get("requestId").is_some() {
            return value;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn installed_javascript_resources_use_readonly_context_cancel_and_bounded_registration_bytes()
{
    tokio::time::timeout(Duration::from_secs(75), scenario())
        .await
        .unwrap();
}
async fn scenario() {
    let fixture = ClientFixture::new("maka-js-context-");
    std::fs::write(
        fixture.workspace.join("proof.txt"),
        "Immutable workspace proof",
    )
    .unwrap();
    let provider = Provider::start().await;
    let model = configure(&fixture, &provider.base_url).await;
    let package = super::super::javascript_plugins::package(
        &fixture.workspace,
        PACKAGE,
        "dedicated",
        include_str!("../../fixtures/input-resources-plugin.mjs"),
        false,
    );
    std::fs::write(
        package.join("context-ui.mjs"),
        include_str!("../../fixtures/context-ui.mjs"),
    )
    .unwrap();
    let host = Host::open(fixture.owner()).await.unwrap();
    #[cfg(unix)]
    let endpoint = fixture
        .workspace
        .parent()
        .unwrap()
        .join("javascript-context.sock");
    #[cfg(windows)]
    let endpoint = std::path::PathBuf::from(format!(
        r"\\.\pipe\maka-js-context-{}",
        uuid::Uuid::new_v4()
    ));
    let stop = CancellationToken::new();
    let cleanup = stop.clone().drop_guard();
    let server = tokio::spawn(
        LocalListener::bind(&endpoint)
            .unwrap()
            .serve(host.clone(), stop.clone()),
    );
    let mut peer = Peer::new(host.clone(), "javascript-context").await;
    let installed = peer
        .rpc("plugin.package.install", json!({"sourcePath":package}))
        .await;
    assert_eq!(installed["ok"], true, "{installed}");
    peer.wait_for_plugins().await;
    let created=peer.rpc("session.create",json!({"sessionId":SESSION,"workspace":{"kind":"host_path","path":fixture.workspace},
        "sandboxMode":"read-only","approvalPolicy":{"kind":"never"},"modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}})).await;
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(remote(&mut peer, "light-383", Value::Null, None).await, 383);
    let directory = peer
        .rpc(
            "plugin.platform.query",
            json!({"view":"input_resources","rootId":format!("session:{SESSION}"),"limit":32}),
        )
        .await;
    assert_eq!(directory["ok"], true, "{directory}");
    let row = directory["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["provider"] == "example.context")
        .unwrap()
        .clone();
    let method = row["method"].as_str().unwrap();
    let first = remote(
        &mut peer,
        method,
        json!({"kind":"query","query":"proof","limit":1,"locale":"en"}),
        Some(SESSION),
    )
    .await;
    assert_eq!(first["nextCursor"], "1");
    let second=remote(&mut peer,method,json!({"kind":"query","query":"proof","cursor":first["nextCursor"],"limit":1,"locale":"en"}),Some(SESSION)).await;
    assert!(second["nextCursor"].is_null());
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
    let resolved = remote(
        &mut peer,
        method,
        json!({"kind":"resolve","id":"proof:0","locale":"en"}),
        Some(SESSION),
    )
    .await;
    assert_eq!(
        resolved["source"]["registration"],
        row["target"]["registration"]
    );
    assert_eq!(resolved["source"]["sessionId"], SESSION);
    assert_eq!(resolved["quote"]["text"], "Immutable workspace proof");
    assert_eq!(
        remote(&mut peer, "stats", Value::Null, None).await["prepared"],
        0
    );
    let opened = peer
        .rpc("plugin.remote", json!({"kind":"open_document"}))
        .await;
    let document = opened["result"]["document"].clone();
    let binding = json!({"packageId":PACKAGE,"method":method,"sessionId":SESSION});
    peer.send_rpc("pending-lookup","plugin.remote",json!({"kind":"call","binding":binding,"target":row["target"],"document":document,"input":{"kind":"query","query":"pending","limit":1,"locale":"en"}}));
    loop {
        if remote(&mut peer, "stats", Value::Null, None).await["pending"] == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    peer.send_rpc(
        "close-lookup",
        "plugin.remote",
        json!({"kind":"close_document","document":document}),
    );
    let replies = [response(&mut peer).await, response(&mut peer).await];
    assert!(
        replies
            .iter()
            .any(|reply| reply["requestId"] == "close-lookup" && reply["ok"] == true),
        "{replies:?}"
    );
    assert!(
        replies
            .iter()
            .any(|reply| reply["requestId"] == "pending-lookup" && reply["ok"] == false),
        "{replies:?}"
    );
    assert_eq!(
        remote(&mut peer, "stats", Value::Null, None).await["cancelled"],
        1
    );
    let budget = remote(&mut peer, "registration-budget", Value::Null, None).await;
    assert_eq!(budget["light"], 320);
    assert!(budget["large"].as_u64().unwrap() > 128, "{budget}");
    assert!(
        budget["failure"]
            .as_str()
            .unwrap()
            .contains("metadata exceeds 32 MiB"),
        "{budget}"
    );
    assert_eq!(budget["reused"], true);
    let disabled = peer
        .rpc(
            "plugin.composition.apply",
            json!({"operations":[{"type":"update","entryId":PACKAGE,"patch":{"disabled":true}}]}),
        )
        .await;
    assert_eq!(disabled["ok"], true, "{disabled}");
    peer.wait_for_plugins().await;
    let retired = peer
        .rpc("plugin.remote", json!({"kind":"bind","binding":binding}))
        .await;
    assert_eq!(retired["ok"], false, "{retired}");
    peer.close().await;
    stop.cancel();
    server.await.unwrap().unwrap();
    cleanup.disarm();
}
