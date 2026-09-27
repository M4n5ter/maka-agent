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

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn cua_bindings_survive_turns_are_session_isolated_and_reset_independently() {
    tokio::time::timeout(Duration::from_secs(45),async {
        let fixture=ClientFixture::new("maka-cua-persistence-");
        let (provider,mut requests)=Provider::controlled().await;
        let model=configure(&fixture,&provider.base_url).await;
        let owner=fixture.owner();
        let database=owner.canonical_path().join(maka_event_log::root::ROOT_DATABASE);
        let host=Host::open(owner).await.unwrap();
        let mut database=SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(database).read_only(true)).await.unwrap();
        let stop=CancellationToken::new();
        let cleanup=stop.clone().drop_guard();
        #[cfg(unix)]
        let endpoint=fixture.workspace.parent().unwrap().join("persistence.sock");
        #[cfg(windows)]
        let endpoint=std::path::PathBuf::from(format!(r"\\.\pipe\maka-cua-{}",uuid::Uuid::new_v4()));
        let server=tokio::spawn(LocalListener::bind(&endpoint).unwrap().serve(host.clone(),stop));
        let mut peer=Peer::new(host,"cua-client").await;
        peer.wait_for_plugins().await;
        for session in ["a","b","approved"] {
            let mode=if session=="approved" {"workspace-write"} else {"danger-full-access"};
            let policy=if session=="approved" {"on-request"} else {"never"};
            assert_eq!(peer.rpc("session.create",json!({"sessionId":session,"sandboxMode":mode,"approvalPolicy":{"kind":policy},"workspace":{"kind":"host_path","path":fixture.workspace},"modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}})).await["ok"],true);
        }
        for (session,turn,tool,input,expected) in [
            ("a","first","cua_repl",json!({"code":"const retained={value:41}; nodeRepl.write(retained.value);"}),"41"),
            ("b","isolated","cua_repl",json!({"code":"nodeRepl.write(typeof retained);"}),"undefined"),
            ("a","next","cua_repl",json!({"code":"retained.value++; nodeRepl.write(retained.value);"}),"42"),
            ("a","reset","cua_reset",json!({}),"reset"),
            ("a","fresh","cua_repl",json!({"code":"nodeRepl.write(typeof retained);"}),"undefined"),
            ("approved","delayed","cua_repl",json!({"code":"await cua.rewriteDocumentation(); nodeRepl.write(43);","timeout_ms":1000}),"43"),
        ] {
            assert_eq!(peer.rpc("turn.start",json!({"sessionId":session,"turnId":turn,"content":{"text":"Exercise only the Computer Use JavaScript environment"}})).await["ok"],true);
            requests.recv().await.unwrap().reply.send(call("search","tool_search",json!({"query":tool}))).unwrap();
            let request=requests.recv().await.unwrap();
            assert!(request.body["tools"].as_array().unwrap().iter().any(|entry|entry["function"]["name"]==tool));
            request.reply.send(call(&format!("cell-{turn}"),tool,input)).unwrap();
            if session=="approved" {
                let id:String=loop {
                    let pending=sqlx::query_scalar("SELECT request_id FROM interaction_requests WHERE json_extract(record_json, '$.sessionId') = ?").bind(session).fetch_optional(&mut database).await.unwrap();
                    if let Some(id)=pending {break id;}
                    tokio::task::yield_now().await;
                };
                // A response before approval would mean the 1 s VM deadline
                // incorrectly consumed time spent waiting for the human.
                assert!(tokio::time::timeout(Duration::from_millis(1200),requests.recv()).await.is_err());
                assert_eq!(peer.rpc("interaction.answer",json!({"sessionId":session,"interactionId":id,"answer":{"kind":"client_capability","decision":"allow"}})).await["ok"],true);
            }
            let request=requests.recv().await.unwrap();
            let output=request.body["messages"].as_array().unwrap().iter().find(|message|message["tool_call_id"]==format!("cell-{turn}")).unwrap()["content"].as_str().unwrap();
            assert!(output.contains(expected),"{session}/{turn}: {output}");
            assert!(!output.contains("tool failed"),"{output}");
            request.reply.send(json!({"index":0,"delta":{"content":"done"},"finish_reason":"stop"})).unwrap();
            loop {
                let state=peer.rpc("turn.query",json!({"sessionId":session,"turnId":turn})).await;
                match state["result"]["status"].as_str() {Some("completed")=>break,Some("failed"|"cancelled")=>panic!("{state}"),_=>tokio::task::yield_now().await}
            }
        }
        peer.close().await; drop(cleanup); server.await.unwrap().unwrap();
    }).await.unwrap();
}
