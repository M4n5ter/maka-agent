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
        let fixture=HostFixture::new("maka-cua-persistence-");
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
        // Keep this lifecycle test hermetic even when the Host runs in WSL.
        assert_eq!(peer.rpc("plugin.composition.apply",json!({"operations":[{"type":"update","entryId":"maka.computer-use","patch":{"config":{"desktop":"local"}}}]})).await["ok"],true);
        peer.wait_for_plugins().await;
        for session in ["a","b","approved"] {
            let mode=if session=="approved" {"workspace-write"} else {"danger-full-access"};
            let policy=if session=="approved" {"on-request"} else {"never"};
            assert_eq!(peer.rpc("session.create",json!({"sessionId":session,"sandboxMode":mode,"approvalPolicy":{"kind":policy},"workspace":{"kind":"host_path","path":fixture.workspace},"modelTarget":{"kind":"explicit","connectionId":model.connection_id,"connectionSlug":model.connection_slug,"model":model.model}})).await["ok"],true);
        }
        for (session,turn,tool,input,expected) in [
            ("a","platform","cua_repl",json!({"code":"nodeRepl.write(cua.computer.target);","title":"检查电脑操作环境"}),if cfg!(target_os="macos") {"mac"} else {std::env::consts::OS}),
            ("a","first","cua_repl",json!({"code":"await cua.cursor.configure({label:'A cursor',color:'blue',enabled:false}); const retained={value:41}; nodeRepl.write(retained.value);"}),"41"),
            ("b","isolated","cua_repl",json!({"code":"nodeRepl.write(typeof retained + ':' + (await cua.cursor.getState()).settings.label);"}),"undefined:Maka"),
            ("a","next","cua_repl",json!({"code":"retained.value++; nodeRepl.write(retained.value + ':' + (await cua.cursor.getState()).settings.label);"}),"42:A cursor"),
            ("a","batch","cua_repl",json!({"code":"for (let i=0;i<40;i++) await cua.cursor.configure({enabled:false}); nodeRepl.write('batch completed');"}),"batch completed"),
            ("a","failure","cua_repl",json!({"code":"nodeRepl.write('observed before failure'); throw new Error('boom');","title":"检查应用"}),"observed before failure"),
            ("a","reset","cua_reset",json!({}),"reset"),
            ("a","fresh","cua_repl",json!({"code":"nodeRepl.write(typeof retained + ':' + (await cua.cursor.getState()).settings.label);"}),"undefined:Maka"),
            ("approved","delayed","cua_repl",json!({"code":"await cua.cursor.configure({label:'approved',enabled:false}); await cua.rewriteDocumentation(); nodeRepl.write(43);","timeout_ms":1000}),"43"),
        ] {
            assert_eq!(peer.rpc("turn.start",json!({"sessionId":session,"turnId":turn,"content":{"text":"Exercise only the Computer Use JavaScript environment"}})).await["ok"],true);
            let search = requests.recv().await.unwrap();
            if turn == "platform" {
                assert!(!search.body["messages"].to_string().contains("Computer Use state captured"), "CUA state is only relevant after CUA becomes callable");
            }
            search.reply.send(call("search","tool_search",json!({"query":tool}))).unwrap();
            let request=requests.recv().await.unwrap();
            assert!(request.body["tools"].as_array().unwrap().iter().any(|entry|entry["function"]["name"]==tool));
            let snapshot = request.body["messages"].to_string();
            let expected_state = if matches!(turn, "platform" | "isolated" | "fresh" | "delayed") { "REPL was fresh" } else { "REPL was running" };
            assert!(snapshot.contains(expected_state), "{session}/{turn}: missing {expected_state}");
            assert!(snapshot.contains("No browser-tab provider is configured"));
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
            let mut request=requests.recv().await.unwrap();
            let mut output=request.body["messages"].as_array().unwrap().iter().find(|message|message["tool_call_id"]==format!("cell-{turn}")).unwrap()["content"].as_str().unwrap().to_owned();
            // First-use API documentation may exceed the inline result budget.
            // Follow the same durable Read continuation exposed to the model;
            // persistence does not require every result to remain inline.
            if let Ok(archive) = serde_json::from_str::<Value>(&output)
                && archive["kind"] == "maka.archived_tool_result"
            {
                let mut page = archive["page"].clone();
                output = page["content"].as_str().unwrap().to_owned();
                let mut page_number = 0;
                while !page["next"].is_null() {
                    page_number += 1;
                    assert!(page_number <= 16, "CUA fixture output must have bounded pages");
                    let id = format!("page-{turn}-{page_number}");
                    request.reply.send(call(&id, "Read", page["next"].clone())).unwrap();
                    request = requests.recv().await.unwrap();
                    let content = request.body["messages"].as_array().unwrap().iter().find(|message|message["tool_call_id"]==id).unwrap()["content"].as_str().unwrap();
                    page = serde_json::from_str(content).unwrap();
                    output.push_str(page["content"].as_str().unwrap());
                }
            }
            assert!(output.contains(expected),"{session}/{turn}: {output}");
            if turn == "failure" { assert!(output.contains("Cua JavaScript error: Error: boom"), "{output}"); }
            else { assert!(!output.contains("tool failed"), "{output}"); }
            request.reply.send(json!({"index":0,"delta":{"content":"done"},"finish_reason":"stop"})).unwrap();
            loop {
                let state=peer.rpc("turn.query",json!({"sessionId":session,"turnId":turn})).await;
                match state["result"]["status"].as_str() {Some("completed")=>break,Some("failed"|"cancelled")=>panic!("{state}"),_=>tokio::task::yield_now().await}
            }
        }
        let title: String = sqlx::query_scalar("SELECT json_extract(event_json,'$.fact.title.fallback') FROM runtime_events WHERE kind='tool_dispatched' AND json_extract(event_json,'$.fact.name')='cua_repl' AND json_extract(event_json,'$.invocation.turn_id')='platform'").fetch_one(&mut database).await.unwrap();
        assert_eq!(title, "检查电脑操作环境");
        let title: String = sqlx::query_scalar("SELECT json_extract(event_json,'$.fact.title.translations.\"zh-CN\"') FROM runtime_events WHERE kind='tool_dispatched' AND json_extract(event_json,'$.fact.name')='cua_operation' AND json_extract(event_json,'$.fact.input.method')='configureCursor' LIMIT 1").fetch_one(&mut database).await.unwrap();
        assert_eq!(title, "设置操作光标");
        let outcome: String = sqlx::query_scalar("SELECT json_extract(event_json,'$.fact.outcome.kind') FROM runtime_events WHERE kind='tool_settled' AND json_extract(event_json,'$.invocation.turn_id')='failure' AND operation_id IN (SELECT operation_id FROM runtime_events WHERE kind='tool_dispatched' AND json_extract(event_json,'$.fact.name')='cua_repl')").fetch_one(&mut database).await.unwrap();
        assert_eq!(outcome, "failed", "a script exception is the same failure for model history, live delivery and transcript projection");
        peer.close().await; drop(cleanup); server.await.unwrap().unwrap();
    }).await.unwrap();
}
