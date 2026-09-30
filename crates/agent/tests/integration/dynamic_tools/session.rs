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
use std::time::Duration;

fn catalog(reverse: bool, effects: Arc<AtomicUsize>) -> ToolCatalog {
    let mut names = vec!["zeta", "echo"];
    if reverse {
        names.reverse();
    }
    ToolCatalog::new(names.into_iter().map(|name| ToolRegistration {
        definition: ToolDefinition { freeform: None, output_schema: None, provider: None,
            name: name.into(), description: "fixture echo tool".into(),
            input_schema: json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}) },
        nesting: ToolNesting::Nestable, semantics: ToolSemantics::Parallel,
        handler: ToolHandler::Immediate(Arc::new(Echo(effects.clone()))),
    })).unwrap().with_discovery()
}
fn input(base: &str, turn: &str, mode: ToolMode, tools: ToolCatalog) -> maka_agent::RunInput {
    let mut input = fixture::input(base, turn, false);
    input.configuration.tool_mode = mode;
    input.work = RunWork::Message {
        allow_prior_unknown: false,
        source_messages: vec![],
        message: "continue".into(),
        tools,
        max_steps: 2,
    };
    input
}
async fn next(listener: &TcpListener) -> (TcpStream, Value) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let request = fixture::read_request(&mut socket).await;
    (socket, request)
}
fn empty(request: &Value, mode: ToolMode) {
    assert_eq!(
        visible(request, mode),
        if mode == ToolMode::Direct {
            vec!["tool_search".to_string()]
        } else {
            vec![]
        }
    );
}
fn loaded(request: &Value, mode: ToolMode) {
    assert_eq!(
        visible(request, mode),
        if mode == ToolMode::Direct {
            vec![
                "echo".to_string(),
                "zeta".to_string(),
                "tool_search".to_string(),
            ]
        } else {
            vec!["echo".to_string(), "zeta".to_string()]
        }
    );
}
fn echo(mode: ToolMode, id: &str) -> Value {
    if mode == ToolMode::Direct {
        call(id, "echo", json!({"n":2}))
    } else {
        call(id, "exec", json!({"code":"text(await tools.echo({n:2}));"}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn session_loading_preserves_wire_prefixes_until_committed_compaction_and_cold_restart() {
    tokio::time::timeout(Duration::from_secs(60), async {
        for mode in [ToolMode::Direct, ToolMode::CodeMode] {
            for success in [true, false] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("events.sqlite");
                let log = Arc::new(EventLog::open(&path).await.unwrap());
                for session in ["session","isolated"] { log.create_session(session,session,&json!({}),1).await.unwrap(); }
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base = format!("http://{}/v1",listener.local_addr().unwrap());
                let server = tokio::spawn(async move {
                    let (mut socket, first) = next(&listener).await;
                    empty(&first, ToolMode::Direct);
                    respond_tools(&mut socket, vec![call("discover", "tool_search", json!({"query":"fixture"}))], 10).await;
                    let (mut socket, seeded) = next(&listener).await;
                    loaded(&seeded, ToolMode::Direct);
                    fixture::respond(&mut socket,"loaded","stop").await;

                    let (mut socket,resumed)=next(&listener).await;
                    assert_eq!(resumed["tools"],seeded["tools"],"read-only continuation preflight cannot erase Session loading preferences");
                    fixture::respond(&mut socket,"continued","stop").await;

                    // New ordinary Turn and freshly constructed catalog, without another search.
                    let (mut socket, warm) = next(&listener).await;
                    loaded(&warm, mode);
                    if mode == ToolMode::Direct { assert_eq!(warm["tools"],seeded["tools"]); }
                    respond_tools(&mut socket,vec![echo(mode,"use")],10).await;
                    let (mut socket, after) = next(&listener).await;
                    assert_eq!(after["tools"],warm["tools"],"actual schema order and declaration text remain unchanged after use");
                    fixture::respond(&mut socket,"done","stop").await;

                    let (mut socket, stable) = next(&listener).await;
                    assert_eq!(stable["tools"],warm["tools"],"registration insertion order and ordinary Turn boundaries cannot change the wire prefix");
                    let body = r#"{"error":{"message":"retry fixture","type":"rate_limit_error"}}"#;
                    socket.write_all(format!("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0.01\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                    let (mut socket,retry)=next(&listener).await;
                    assert_eq!(retry,stable,"physical retry must preserve the complete request, including the loaded prefix");
                    fixture::respond(&mut socket,"recovered","stop").await;

                    let (mut socket,other)=next(&listener).await;
                    empty(&other,mode);
                    fixture::respond(&mut socket,"isolated","stop").await;
                    for _ in 0..if success {1} else {2} {
                        let (mut socket,summary)=next(&listener).await;
                        assert!(summary.get("tools").is_none());
                        fixture::respond(&mut socket,if success {fixture::SUMMARY} else {"invalid summary"},"stop").await;
                    }
                    let (mut socket,compacted)=next(&listener).await;
                    if success {empty(&compacted,mode);} else {assert_eq!(compacted["tools"],warm["tools"],"a failed summary cannot unload anything");}
                    if mode == ToolMode::CodeMode {
                        // Omitted descriptions do not revoke nested callability.
                        respond_tools(&mut socket,vec![echo(mode,"after-compact")],10).await;
                        let (mut socket,after)=next(&listener).await;
                        assert_eq!(after["tools"],compacted["tools"]);
                        fixture::respond(&mut socket,"done","stop").await;
                    } else { fixture::respond(&mut socket,"done","stop").await; }
                    let (mut socket,reopened)=next(&listener).await;
                    empty(&reopened,mode);
                    fixture::respond(&mut socket,"fresh host","stop").await;
                });
                let effects=Arc::new(AtomicUsize::new(0));
                let engine=fixture::engine(log.clone());
                engine.run(input(&base,"seed",ToolMode::Direct,catalog(false,effects.clone())),CancellationToken::new()).await.unwrap();
                // Read-only continuation previews use a detached preference snapshot.
                use maka_runtime::event::{EventWrite, InvocationOutcome, RuntimeEvent};
                use maka_runtime::input::InvocationInput;
                let mut source=input(&base,"preview-source",ToolMode::Direct,catalog(false,effects.clone()));
                source.configuration.workspace_identity=Some(maka_runtime::execution::WorkspaceIdentity::from_marker_id("c193cd58-f929-4ba2-bfb5-6887beaf8132").unwrap());
                for fact in [
                    Fact::InvocationOpened { configuration:Some(Box::new(source.configuration.clone())), input:InvocationInput::Message {content:"continue".into(),request_fingerprint:None,source_messages:vec![]} },
                    Fact::InvocationEnded {outcome:InvocationOutcome::Cancelled {source:"user".into()}},
                ] {log.append(&EventWrite::plain(RuntimeEvent::new(source.invocation.clone(),fact)).unwrap()).await.unwrap();}
                let history=log.run_prefix("session",&source.invocation.run_id,None,100,128*1024).await.unwrap().unwrap();
                let boundary=maka_runtime::continuation::RunBoundary {invocation:history.invocation,high_water:history.high_water,digest:history.digest};
                let mut resumed=input(&base,"resumed",ToolMode::Direct,catalog(false,effects.clone()));
                resumed.configuration=source.configuration.clone();
                resumed.request_fingerprint=Some(maka_runtime::artifact::content_digest(b"resume-fixture"));
                resumed.work=RunWork::Continuation {source:boundary.clone(),tools:catalog(true,effects.clone()),max_steps:1};
                let mut preview=resumed.clone();
                preview.work=RunWork::Continuation {source:boundary,tools:ToolCatalog::default().with_discovery(),max_steps:1};
                assert!(matches!(engine.check_continuation(&preview,&CancellationToken::new()).await,
                    Err(maka_agent::RunError::ReconciliationRequired(message)) if message.contains("unavailable tool")));
                engine.run(resumed,CancellationToken::new()).await.unwrap();
                engine.run(input(&base,"warm",mode,catalog(true,effects.clone())),CancellationToken::new()).await.unwrap();
                engine.run(input(&base,"stable",mode,catalog(false,effects.clone())),CancellationToken::new()).await.unwrap();

                let mut isolated=input(&base,"isolated",mode,catalog(true,effects.clone()));
                isolated.invocation.session_id="isolated".into();
                engine.run(isolated,CancellationToken::new()).await.unwrap();
                engine.run(fixture::input(&base,"compact",true),CancellationToken::new()).await.unwrap();
                engine.run(input(&base,"after-compact",mode,catalog(true,effects.clone())),CancellationToken::new()).await.unwrap();
                engine.drain().await;
                let prefix=log.prefix(300,2*1024*1024).await.unwrap();
                assert_eq!(prefix.events.iter().filter(|event|matches!(&event.event.fact,Fact::ToolDispatched {name,..} if name=="tool_search")).count(),1);
                assert_eq!(prefix.events.iter().filter(|event|matches!(event.event.fact,Fact::ContextCheckpointRecorded {..})).count(),usize::from(success));
                assert_eq!(effects.load(Ordering::SeqCst),if mode==ToolMode::CodeMode {2} else {1});
                let claim=prefix.events.iter().find_map(|event|match &event.event.fact {Fact::InvocationOpened {input:InvocationInput::Continuation {claim,..},..}=>Some(claim),_=>None}).unwrap();
                let actual=prefix.events.iter().find_map(|event|match &event.event.fact {Fact::ModelRequested {input_digest,..} if event.event.invocation.run_id=="run-resumed"=>Some(input_digest),_=>None}).unwrap();
                assert_eq!(actual,&claim.replay.digest,"warm continuation proof matches the real request");
                drop(engine);
                Arc::try_unwrap(log).ok().unwrap().close().await.unwrap();
                let log=Arc::new(EventLog::open(&path).await.unwrap());
                let reopened=fixture::engine(log);
                reopened.run(input(&base,"restart",mode,catalog(false,effects)),CancellationToken::new()).await.unwrap();
                reopened.drain().await;
                server.await.unwrap();
            }
        }
    }).await.expect("Session discovery and compaction must make bounded progress");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn continuation_admission_rejects_changed_loading_before_claiming_the_source() {
    use maka_runtime::{
        continuation::RunBoundary,
        event::{EventWrite, InvocationOutcome, RuntimeEvent},
        input::InvocationInput,
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        for mode in [ToolMode::Direct, ToolMode::CodeMode] {
            let directory = tempfile::tempdir().unwrap();
            let log = Arc::new(
                EventLog::open(&directory.path().join("events.sqlite"))
                    .await
                    .unwrap(),
            );
            log.create_session("session", "session", &json!({}), 1)
                .await
                .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, request) = next(&listener).await;
                empty(&request, ToolMode::Direct);
                respond_tools(
                    &mut socket,
                    vec![call("discover", "tool_search", json!({"query":"fixture"}))],
                    10,
                )
                .await;
                let (mut socket, request) = next(&listener).await;
                loaded(&request, ToolMode::Direct);
                fixture::respond(&mut socket, "loaded", "stop").await;
                let (mut socket, summary) = next(&listener).await;
                assert!(summary.get("tools").is_none());
                fixture::respond(&mut socket, fixture::SUMMARY, "stop").await;
                let (mut socket, resumed) = next(&listener).await;
                empty(&resumed, mode);
                fixture::respond(&mut socket, "continued", "stop").await;
            });
            let effects = Arc::new(AtomicUsize::new(0));
            let engine = fixture::engine(log.clone());
            let mut source = input(&base, "source", mode, catalog(false, effects.clone()));
            source.configuration.workspace_identity = Some(
                maka_runtime::execution::WorkspaceIdentity::from_marker_id(
                    "c193cd58-f929-4ba2-bfb5-6887beaf8132",
                )
                .unwrap(),
            );
            for fact in [
                Fact::InvocationOpened {
                    configuration: Some(Box::new(source.configuration.clone())),
                    input: InvocationInput::Message {
                        content: "continue".into(),
                        request_fingerprint: None,
                        source_messages: vec![],
                    },
                },
                Fact::InvocationEnded {
                    outcome: InvocationOutcome::Cancelled {
                        source: "user".into(),
                    },
                },
            ] {
                log.append(
                    &EventWrite::plain(RuntimeEvent::new(source.invocation.clone(), fact)).unwrap(),
                )
                .await
                .unwrap();
            }
            let prefix = log
                .run_prefix("session", &source.invocation.run_id, None, 100, 128 * 1024)
                .await
                .unwrap()
                .unwrap();
            let mut resumed = source.clone();
            resumed.invocation = input(&base, "resumed", mode, ToolCatalog::default()).invocation;
            resumed.request_fingerprint =
                Some(maka_runtime::artifact::content_digest(b"resume-fixture"));
            resumed.work = RunWork::Continuation {
                source: RunBoundary {
                    invocation: prefix.invocation,
                    high_water: prefix.high_water,
                    digest: prefix.digest,
                },
                tools: catalog(false, effects.clone()),
                max_steps: 1,
            };
            let cold = engine
                .prepare_continuation(resumed.clone(), &CancellationToken::new())
                .await
                .unwrap();
            engine
                .run(
                    input(&base, "seed", ToolMode::Direct, catalog(false, effects)),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let before = log.prefix(200, 1024 * 1024).await.unwrap();
            assert!(matches!(
                engine
                    .start_continuation(cold, CancellationToken::new())
                    .await,
                Err(maka_agent::RunError::ContinuationChanged)
            ));
            assert_eq!(
                log.prefix(200, 1024 * 1024).await.unwrap().digest,
                before.digest,
                "stale cold preparation cannot open or claim a Run"
            );

            let warm = engine
                .prepare_continuation(resumed.clone(), &CancellationToken::new())
                .await
                .unwrap();
            engine
                .run(
                    fixture::input(&base, "compact", true),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let before = log.prefix(200, 1024 * 1024).await.unwrap();
            assert!(matches!(
                engine
                    .start_continuation(warm, CancellationToken::new())
                    .await,
                Err(maka_agent::RunError::ContinuationChanged)
            ));
            assert_eq!(
                log.prefix(200, 1024 * 1024).await.unwrap().digest,
                before.digest,
                "stale warm preparation cannot restore names cleared by compaction"
            );

            let current = engine
                .prepare_continuation(resumed, &CancellationToken::new())
                .await
                .unwrap();
            engine
                .start_continuation(current, CancellationToken::new())
                .await
                .unwrap()
                .wait()
                .await
                .unwrap();
            engine.drain().await;
            server.await.unwrap();
            let prefix = log.prefix(200, 1024 * 1024).await.unwrap();
            let claim = prefix
                .events
                .iter()
                .find_map(|event| match &event.event.fact {
                    Fact::InvocationOpened {
                        input: InvocationInput::Continuation { claim, .. },
                        ..
                    } => Some(claim),
                    _ => None,
                })
                .unwrap();
            let actual = prefix
                .events
                .iter()
                .find_map(|event| match &event.event.fact {
                    Fact::ModelRequested { input_digest, .. }
                        if event.event.invocation.run_id == "run-resumed" =>
                    {
                        Some(input_digest)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                actual, &claim.replay.digest,
                "freshly admitted evidence matches the actual cold request"
            );
        }
    })
    .await
    .expect("stale continuation preparation must be retryable without effects");
}
