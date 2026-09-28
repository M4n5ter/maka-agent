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

# Plugin capability map

Inspect these source contracts and their consumers before changing a feature.

| Need | Contract | Example |
| --- | --- | --- |
| Composition and live ownership | `crates/plugins/src/composition/ledger.rs`, `crates/plugins/src/kernel.rs`, `crates/plugins/src/fiber.rs` | `crates/runtime-host/src/plugins/owner.rs` |
| Model-visible tools | `maka_tool_catalog::plugins::PluginTool`; `ctx.tools` | `crates/web/src/plugin.rs` |
| Input preparation | `maka_plugins::input`; `ctx.input` | `crates/skills/src/plugin.rs` |
| Session behavior | `maka_plugins::session::SessionBehavior`; `ctx.behaviors` | `crates/graph/src/plugin.rs` |
| Model adapters | `maka_plugins::model`; `ctx.modelAdapters` | `crates/model/src/adapters.rs` |
| External executors | `maka_plugins::executor`; `ctx.executors` | `crates/runtime-host/tests/fixtures/workflow/host.mjs` |
| Remote calls and streams | `maka_plugins::remote::Endpoint`; `ctx.remote` | `crates/goal/src/plugin/remote.rs` |
| Background work | `maka_plugins::background::BackgroundWork`; `ctx.background` | `crates/scheduler/src/plugin.rs` |
| Declarative views | `maka_plugins::terminal_ui`; `ctx.tui.app` | `crates/goal/src/plugin/terminal.rs`, `crates/cli/tests/fixtures/board-plugin/` |

Host capabilities live in `crates/plugins/src/host.rs` and `packages/plugin-sdk/src/host.ts`. They are issued to a Fiber owner; they do not expose the private Host object.

- Storage and preferences are namespaced by package and scope.
- Credentials and authorization stay in Host-owned services. Discovery and read views do not grant execution.
- Execution receipts remain canonical Host facts after plugin retirement.
- Files, HTTP, processes and terminals retain scoped permissions, cancellation and resource settlement.
- Remote bindings pin a registration and document lifetime. Methods accepting raw Host paths declare that requirement explicitly.

The desired plugin tree, live Fiber and callable contribution snapshot are separate states. A successful composition write does not prove activation succeeded; each call still checks that its captured registration is effective.
