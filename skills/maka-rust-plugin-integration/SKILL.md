---
name: maka-rust-plugin-integration
description: Implement or review Maka plugins using the Rust Runtime Host capabilities, the public Host SDK, and declarative terminal views.
license: Apache-2.0
---
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

# Maka plugin integration

Use the owning crate and public SDK as the API authority. Source paths below are relative to the repository root; inspect the relevant consumer before extending a contract.

## Choose the owner

Keep domain policy and durable intent in the plugin. Host owns Session/Turn identity, admission, event facts, credentials and resource settlement. A visible method or view does not grant execution or filesystem authority.

Use native Rust for built-ins and the Host SDK for installable JavaScript packages. A Session capture overlays Profile contributions. Read [the capability map](references/capability-map.md) when choosing the contribution or Host service.

## Integrate the feature

- Native built-ins implement `maka_plugins::kernel::Plugin` and publish typed `Staged` contributions. Register their `Definition` and composition Entry in `crates/runtime-host/src/plugins/`. `crates/web/src/plugin.rs` is a small example; Graph adds Session behavior and Scheduler owns durable background work.
- External packages use `@maka-agent/plugin-sdk/host`. Follow `crates/cli/tests/fixtures/board-plugin/` for a complete package with a Host entry and declarative view factories. The immutable manifest pins the SDK version and entrypoints.
- Declare views through `ctx.tui.app`; view factories receive pure builders while business callbacks retain scoped Host capabilities. For native presentation, follow an owning crate's `plugin/terminal.rs`.
- Use `context.lifecycle` in Rust or the scoped JS context for tasks and cleanup. Persist intent before effects, use stable operation IDs, and reconcile uncertain results through the original receipt. Stopping observation does not cancel accepted Host work.

## Verify the consumer

Exercise composition, activation, admission and the actual effect or view, then verify retirement and recovery where they matter. Partial activation must not publish half a contribution batch, and retired registrations must reject new calls.

Run focused `just test -p <crate>` checks and `just typecheck` for SDK or JavaScript fixture changes. Use `just test-js` for the SDK's runtime contracts. If the public capability is missing, identify the narrow contract and a concrete consumer instead of passing a private Host handle into a plugin.
