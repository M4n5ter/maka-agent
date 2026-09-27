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

//! An isolated, persistent JavaScript REPL. This is independent of Code Mode:
//! no modules, Node, network inspector, filesystem or trusted plugin powers.
//! The caller owns its lifetime and supplies fresh authority for each evaluation.

use crate::{
    CellAbort, CellContext, CellDiagnostic, CellDiagnosticKind, CellLimits, CellResult,
    bridge::{Admission, ToolScope, maka_code},
    result::bounded,
};
use deno_core::{
    InspectorMsg, InspectorMsgKind, InspectorSessionKind, JsRuntime, JsRuntimeInspector,
    RuntimeOptions, v8,
};
use maka_runtime::tools::ToolExecutor;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc, oneshot},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub struct Repl {
    sender: mpsc::Sender<Evaluation>,
    stopped: CancellationToken,
    worker: tokio::task::JoinHandle<Result<(), CellAbort>>,
}

struct Evaluation {
    source: String,
    executor: Arc<dyn ToolExecutor>,
    cancellation: CancellationToken,
    context: CellContext,
    timeout: std::time::Duration,
    reply: oneshot::Sender<Result<CellResult, CellAbort>>,
}

impl Repl {
    /// `bootstrap` is trusted facade source, evaluated once. It can capture the
    /// two bridge functions `call(name, input)` and `emit(CellOutput)`; these
    /// are not globals and contain no persistent invocation authority.
    pub fn new(bootstrap: &'static str, limits: CellLimits) -> Self {
        crate::initialize_platform();
        let (sender, receiver) = mpsc::channel(1);
        let stopped = CancellationToken::new();
        let stop = stopped.clone();
        let host = Handle::current();
        let worker = tokio::task::spawn_blocking(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| CellAbort::Internal(e.to_string()))?
                .block_on(serve(bootstrap, limits, receiver, stop, host))
        });
        Self {
            sender,
            stopped,
            worker,
        }
    }

    pub async fn evaluate(
        &self,
        source: String,
        executor: Arc<dyn ToolExecutor>,
        cancellation: CancellationToken,
        context: CellContext,
        timeout: std::time::Duration,
    ) -> Result<CellResult, CellAbort> {
        let cancellation = cancellation.child_token();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let (reply, receive) = oneshot::channel();
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(CellAbort::Cancelled),
            result = self.sender.send(Evaluation { source, executor, cancellation: cancellation.clone(), context, timeout, reply }) => {
                result.map_err(|_| closed())?;
            }
        }
        // Do not race cancellation against settlement of an admitted evaluation.
        receive.await.map_err(|_| closed())?
    }

    pub async fn close(self) -> Result<(), CellAbort> {
        self.stopped.cancel();
        self.worker
            .await
            .map_err(|e| CellAbort::Internal(e.to_string()))?
    }
}

fn closed() -> CellAbort {
    CellAbort::Internal("REPL is closed; reset it before evaluating more code".into())
}

async fn serve(
    bootstrap: &'static str,
    limits: CellLimits,
    mut receiver: mpsc::Receiver<Evaluation>,
    stopped: CancellationToken,
    host: Handle,
) -> Result<(), CellAbort> {
    let mut runtime = JsRuntime::try_new(RuntimeOptions {
        inspector: true,
        extensions: vec![maka_code::init()],
        create_params: Some(v8::CreateParams::default().heap_limits(0, limits.heap_bytes)),
        ..Default::default()
    })
    .map_err(|e| CellAbort::Internal(e.to_string()))?;
    runtime.execute_script("maka:repl/bootstrap", format!(r#"(() => {{
        const call = Deno.core.ops.op_maka_tool;
        const emit = Deno.core.ops.op_maka_emit;
        for (const name of ["Deno", "console", "Atomics", "SharedArrayBuffer", "WebAssembly", "__bootstrap"])
            delete globalThis[name];
        {bootstrap}
    }})()"#)).map_err(|e| CellAbort::Internal(e.to_string()))?;

    let isolate = runtime.v8_isolate().thread_safe_handle();
    let heap_exhausted = CancellationToken::new();
    let heap_stop = heap_exhausted.clone();
    let heap_isolate = isolate.clone();
    runtime.add_near_heap_limit_callback(move |current, _| {
        heap_stop.cancel();
        heap_isolate.terminate_execution();
        current.saturating_add(16 * 1024 * 1024)
    });
    let (replies, mut responses) = mpsc::unbounded_channel();
    let mut inspector = JsRuntimeInspector::create_local_session(
        runtime.inspector(),
        Box::new(move |msg: InspectorMsg| {
            if let InspectorMsgKind::Message(_) = msg.kind {
                let _ = replies.send(msg.content);
            }
        }),
        InspectorSessionKind::NonBlocking {
            wait_for_disconnect: false,
        },
    );
    let mut sequence = 0;
    loop {
        let request = tokio::select! {
            biased;
            _ = stopped.cancelled() => break,
            request = receiver.recv() => match request { Some(request) => request, None => break },
        };
        if request.cancellation.is_cancelled() {
            let _ = request.reply.send(Err(CellAbort::Cancelled));
            continue;
        }
        if request.source.len() > limits.max_source_bytes || request.timeout.is_zero() {
            let _ = request.reply.send(Ok(bounded(
                Err(CellDiagnostic::limit("invalid REPL source size or timeout")),
                vec![],
                limits.max_value_bytes,
            )));
            continue;
        }
        let scope = Arc::new(ToolScope {
            context: request.context.clone(),
            names: request.executor.names().into_iter().collect(),
            executor: request.executor,
            cancellation: request.cancellation.child_token(),
            tasks: TaskTracker::new(),
            host: host.clone(),
            limits: limits.clone(),
            admission: Mutex::new(Admission::default()),
            concurrency: Arc::new(Semaphore::new(1)),
        });
        runtime.op_state().borrow_mut().put(scope.clone());
        runtime.op_state().borrow_mut().put(request.context.clone());

        let finished = CancellationToken::new();
        let finish = finished.clone();
        let cancellation = request.cancellation.clone();
        let stop = stopped.clone();
        let terminate = isolate.clone();
        let interrupted = CancellationToken::new();
        let interrupt = interrupted.clone();
        let timeout = request.timeout;
        let monitor = host.spawn(async move {
            tokio::select! {
                biased;
                _ = finish.cancelled() => return,
                _ = cancellation.cancelled() => {},
                _ = stop.cancelled() => {},
                _ = tokio::time::sleep(timeout) => {},
            }
            interrupt.cancel();
            terminate.terminate_execution();
        });
        sequence += 1;
        // Local inspector transport gives real REPL lexical bindings and await;
        // there is deliberately no inspector socket or arbitrary CDP endpoint.
        inspector.post_message(
            sequence,
            "Runtime.evaluate",
            Some(json!({
                "expression":request.source,
                "replMode":true, "awaitPromise":true, "returnByValue":false,
                "objectGroup":"maka-repl-result"
            })),
        );
        let mut completed = false;
        let result = tokio::select! {
            biased;
            _ = interrupted.cancelled() => Err(CellDiagnostic::limit("REPL interrupted; reset required")),
            _ = heap_exhausted.cancelled() => Err(CellDiagnostic::limit("JavaScript heap; reset required")),
            result = async {
                runtime.run_event_loop(Default::default()).await
                    .map_err(|e| CellDiagnostic::new(CellDiagnosticKind::ExecutionError, e.to_string()))?;
                let response = responses.recv().await.ok_or_else(|| CellDiagnostic::new(CellDiagnosticKind::ExecutionError, "inspector closed"))?;
                let value: Value = serde_json::from_str(&response)
                    .map_err(|e| CellDiagnostic::new(CellDiagnosticKind::ExecutionError, e.to_string()))?;
                if let Some(error) = value.get("error") {
                    return Err(CellDiagnostic::new(CellDiagnosticKind::ExecutionError,
                        error["message"].as_str().unwrap_or("inspector evaluation failed")));
                }
                let result = value.get("result").filter(|result| result.is_object())
                    .ok_or_else(|| CellDiagnostic::new(CellDiagnosticKind::ExecutionError, "inspector response missing result"))?;
                // A regular JavaScript exception still completed the evaluation.
                completed = true;
                if let Some(error) = result.get("exceptionDetails") {
                    let message = error["exception"]["description"].as_str()
                        .or_else(|| error["text"].as_str()).or_else(|| error["message"].as_str())
                        .unwrap_or("JavaScript execution failed");
                    Err(CellDiagnostic::new(CellDiagnosticKind::ExecutionError, message))
                } else { Ok(Value::Null) }
            } => result,
        };
        // Drain native work even after a JS exception, timeout or a lost caller.
        scope.cancellation.cancel();
        scope.tasks.close();
        scope.tasks.wait().await;
        finished.cancel();
        monitor
            .await
            .map_err(|e| CellAbort::Internal(e.to_string()))?;
        let poisoned = !completed || interrupted.is_cancelled() || heap_exhausted.is_cancelled();
        let result = if let Some(error) = request.context.failure() {
            Err(CellAbort::Tool(error))
        } else if request.cancellation.is_cancelled() || stopped.is_cancelled() {
            Err(CellAbort::Cancelled)
        } else {
            Ok(bounded(
                result,
                scope.admission.lock().unwrap().calls.clone(),
                limits.max_value_bytes,
            ))
        };
        let _ = request.reply.send(result);
        if poisoned {
            break;
        }
        // Release inspector's completion-value references, not user bindings.
        inspector.post_message(
            0,
            "Runtime.releaseObjectGroup",
            Some(json!({"objectGroup":"maka-repl-result"})),
        );
        let _ = responses.recv().await;
        runtime.op_state().borrow_mut().take::<Arc<ToolScope>>();
        runtime.op_state().borrow_mut().take::<CellContext>();
    }
    Ok(())
}
