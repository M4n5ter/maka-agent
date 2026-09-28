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

# Architecture

[简体中文](ARCHITECTURE.zh-CN.md)

Maka has one Rust Runtime Host. The CLI and TUI connect through the public client protocol; neither owns agent execution or durable state.

- **Admission and settlement:** `crates/runtime-host` admits operations, captures execution boundaries and coordinates resource cleanup. `crates/event-log` owns durable execution facts and state-root ownership.
- **Agent execution:** `crates/agent`, `crates/runtime` and `crates/model` build model requests and settle tool effects. `crates/providers` owns provider-specific authentication, discovery and policy; `crates/responses` owns native Responses transport.
- **Capabilities:** business plugins use `crates/plugins` capabilities for storage, models, execution, credentials, HTTP and presentation. Host owns authority and lifetimes; each plugin owns its domain policy.
- **JavaScript:** `crates/js-runtime` embeds V8 for Code Mode and JavaScript plugins. Provider SDKs are bundled at build time. Guest code reaches host capabilities through explicit bindings.
- **Computer Use:** `crates/computer-use` owns its own REPL and session lifetime. It works independently of Code Mode and supports native application and browser surfaces.
- **Presentation:** `crates/client` implements the client protocol. `crates/tui` renders native terminal views, while the plugin SDK describes capabilities and declarative views without owning host state.

The npm package in `packages/cli` only chooses and launches a matching executable. It does not add a second runtime, configuration store or tool implementation.

See the owning crate READMEs for detailed contracts, and [the SDK](packages/plugin-sdk/README.md) for JavaScript plugin authoring.
