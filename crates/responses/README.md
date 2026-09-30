<!--
  Licensed to the Apache Software Foundation (ASF) under one
  or more contributor license agreements.  See the NOTICE file
  distributed with this work for additional information
  regarding copyright ownership.  The ASF licenses this file
  to you under the Apache License, Version 2.0 (the
  "License"); you may not use this file except in compliance
  with the License.  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing,
  software distributed under the License is distributed on an
  "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
  KIND, either express or implied.  See the License for the
  specific language governing permissions and limitations
  under the License.
-->

# Responses

[简体中文](README.zh-CN.md)

Native Rust Responses encoding, stream decoding, and disposable WebSocket continuation state. No V8 dependency. Host owns credentials, request admission, canonical history, and usage settlement.

Inspired by [OpenAI Codex](https://github.com/openai/codex/tree/94174e44cbc54cece45f6052328ca0c2cd7a8a2a/codex-rs/codex-api). HTTP and WebSocket share one event decoder; connection caches are optimizations, not replay authorities.

Outbound model requests have no fixed local body-size cap. Context selection and provider limits govern valid requests; response/event limits remain independent. A request larger than the WebSocket cache budget still runs, without retaining a continuation baseline. The trusted JavaScript adapter reserves its entire input queue window for a larger request instead of rejecting it. Media is validated and read per artifact, so historical images do not crowd out newer images with a cumulative byte allowance.

Native Responses declares provider-confirmed item boundaries. Host marks those requests for incremental acceptance: a complete validated item commits as `ModelObserved` before its local tool can run. Tool effects run with bounded concurrency while the stream continues; exclusive-step admission stays ordered. Code Mode retains its existing cell lifecycle and bounds, with a shared execution gate for nested tools. Adapters without confirmed item boundaries retain whole-response acceptance, as some SDKs synthesize end events while cleaning up a broken stream. Older journals retain their original semantics.

Receive-side WebSocket interruptions carry adapter-owned retry evidence into the Agent's single bounded retry loop. Without accepted progress, retries keep the frozen input. With accepted progress and explicit retained-output safety, Host drains admitted tools and rebuilds input from the committed items and results, retaining the same captured model and tool capabilities. Partial arguments never execute. Unknown provider-side effects, malformed frames and ambiguous sends do not authorize retry; complete opaque metadata is retained rather than discarded. Close status and transport errors remain visible in diagnostics.

The execution and recovery split follows [Codex's complete-item dispatch](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/stream_events_utils.rs), [tool scheduling](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/tools/parallel.rs) and [stream retry owner](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/responses_retry.rs). WS reconnects first with exponential backoff; after nine retries the Agent selects HTTP fallback and resets its bounded budget. Retry-after and cancellation still apply. Fallback retains the existing five-minute route cooldown. Pre-dispatch upgrade negotiation keeps its own bounded attempts before any model request is sent. Explicit policy/protocol close codes remain terminal; a size close (1009) may recover over HTTP after retry exhaustion.
