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

# Native Rust plugins in the Maka source tree

External packages use the JavaScript Host SDK. A native built-in is compiled into
Maka; it is not a dynamic Rust library placed in an extension directory.

Implement `maka_plugins::kernel::Plugin`: supports_scope, validate, and activate.
Activation receives PluginContext and configuration and returns a complete Staged
contribution batch. Use context.host capabilities and context.data when provided,
and context.lifecycle for admissions, tasks, cleanup and retirement. Missing
required capabilities are activation failures, not opportunities to open private
Host paths or retain a Host object.

Register a Definition (ID, revision, dependencies, injection and plugin) and its
composition Entry through `crates/runtime-host/src/plugins/`. Follow the existing
owning module instead of introducing a second registry. Profile and Session capture
overlay typed contributions; publication is separate from desired composition.

| Feature | Owning contracts / implementation example |
| --- | --- |
| Kernel, lifetime, tree and service lookup | `crates/plugins/src/kernel.rs`, `fiber.rs`, `composition/`, `services.rs`; Host `plugins/owner.rs` |
| Host capability issuance | `crates/plugins/src/host.rs`; Host `plugins/authority.rs` |
| Tools and per-step bindings | `maka_tool_catalog::plugins::PluginTool`, BindingProvider; `crates/web/src/plugin.rs` |
| Input preparation and revisions | `maka_plugins::input`, `revision`; `crates/skills/src/plugin.rs` |
| Managed behavior | `maka_plugins::session::SessionBehavior`; `crates/graph/src/plugin.rs` |
| Model protocols and providers | `maka_plugins::model`, `provider`; `crates/model/src/adapters.rs`, `crates/providers/src/` |
| Executors | `maka_plugins::executor`; `crates/external-agent/src/` |
| Remote endpoints | `maka_plugins::remote::Endpoint`; `crates/goal/src/plugin/remote.rs` |
| Background ownership | `maka_plugins::background::BackgroundWork`; `crates/scheduler/src/plugin.rs` |
| Terminal apps | `maka_plugins::terminal_ui`; owning crate `plugin/terminal.rs` |

ToolPreparer parses/approves without admitting the effect. Return PreparedEffect
for the journal-owned execution after durable dispatch. Retain exact call/owner
identity through settlement; retirement rejects new starts but drains accepted
resources. Plugin domain state and stable receipts must survive reactivation when
the feature claims recovery. Prompt capture and CUA/REPL lifetimes remain separate
from description-loading state.

Use `Endpoint::standalone` for Remote and declare host-path access when accepting
raw Host paths. Backend observations must be checked through the supplied read or
execution view. A terminal node's target and an opaque registration token never
substitute for domain authorization.

Inspect the actual source consumer before extending a public contract. Update the
public TS contract and bridge only when JS consumers need the same capability;
add a real consumer test. Run the affected crate tests/Clippy and relevant SDK
contracts. Do not copy a stale trait signature from a guide instead of the owner.
