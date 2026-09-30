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

Receive-side WebSocket interruptions carry decoder-derived replay safety into the Agent’s existing bounded retry loop. The failed connection is discarded; retries reconnect WebSocket with the same frozen model input and exponential backoff. Only after all nine stream retries fail does the Agent select HTTP fallback and reset its bounded retry budget. Server retry-after advice and cancellation still apply. Malformed frames, ambiguous send failures, cancellation and observed provider-side tool effects or opaque replay metadata do not authorize replay. Close status and transport errors remain visible in diagnostics.

The recovery split follows [Codex’s stream retry owner](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/responses_retry.rs) and [transport reset](https://github.com/openai/codex/blob/b1e72963c3b71a9265a551e54beff078384efed9/codex-rs/core/src/client.rs): the transport reports failure; the Agent owns replay. Maka retries WebSocket before switching transport, retaining its existing five-minute bounded route cooldown once fallback is selected. Pre-dispatch upgrade negotiation keeps its separate bounded retries; no model request has been sent at that point. Explicit policy/protocol close codes remain terminal; a size close (1009) can eventually recover over HTTP. Unlike Codex’s item-level acceptance, Maka accepts local tool calls only after the complete model response; retries retain the frozen canonical cut and cannot execute partial calls. Opaque replay carriers keep the existing conservative history boundary.
