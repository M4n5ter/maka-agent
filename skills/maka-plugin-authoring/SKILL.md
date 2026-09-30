---
name: maka-plugin-authoring
description: Create, extend, debug, test, or package Maka plugins for the Rust Runtime Host, including tools, input providers, model providers/adapters, executors, background services, and declarative TUI apps. Use for Maka plugin development and SDK integration.
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

# Maka plugin authoring

Build against the actual Host SDK, keeping plugin policy in the plugin and execution,
permissions, credentials and durable receipts with their Host owners. Prefer an
installable JavaScript package for an external plugin; use native Rust when changing
a compiled-in feature in a Maka source checkout.

## Read the right material

Load this skill with `Skill({name: "maka-plugin-authoring"})`. Read a linked file with:

```json
{"name":"maka-plugin-authoring","resource":{"path":"references/quickstart.md"}}
```

Resources are relative to this skill, not the workspace. If `page.next` is non-null,
pass that entire object to **Skill** unchanged. Use `offset`/`limit` for a relevant
line range. Resources include exact SDK source contracts shipped with this Host;
they are not fetched from an unversioned website. Read only what the task needs.
In a source checkout, the same guides are ordinary relative files; the SDK source
lives at `packages/plugin-sdk/src/` and is embedded as `references/sdk/` when built.

| Task | Read |
| --- | --- |
| First plugin, manifest, ESM bundle, local installation | [Quickstart](references/quickstart.md) |
| Tool declarations, Code Mode, prompts, prepared input and resource discovery | [Tools and input](references/tools-and-input.md) |
| Permissions, call scopes, storage, credentials, cancellation and background work | [State and authority](references/state-and-authority.md) |
| TUI pages, settings, inspector/status panels, forms, streams and recovery | [Terminal apps](references/terminal-apps.md) |
| Model connections/adapters, executors and Session orchestration | [Models and execution](references/models-and-execution.md) |
| Exact method signatures and owning modules | [API map](references/api-map.md) |
| Rust built-ins and contribution ownership | [Native Rust](references/native-rust.md) |
| Validation, upgrades, debugging, package export | [Testing and shipping](references/testing-and-shipping.md) |

## Deliver the requested plugin

1. Identify its contribution and entry scope. Read the relevant guide and SDK module
   before writing calls; the SDK map covers the complete shipped public JS surface.
2. Adapt the runnable starter in `assets/starter/`. Give package, entry, tool and
   view identifiers stable names. Choose only the capabilities the feature needs.
3. Keep effectful business logic in the Host activation/backend. Use the issued
   call context for files, network, processes, models and executions. UI factories
   describe views; they do not retain business authority.
4. Build a complete package, exercise its actual registration and consumer in a
   disposable Host, and test the failure/restart paths that its effects require.
   Confirm live activation, not just successful package installation.
5. Deliver the package and the tested install/update steps. Report any unsupported
   capability directly; do not substitute private Host access or invent SDK methods.

## Contracts that affect every implementation

- `schemaVersion: 1`, `runtime.sdkVersion: 3`; `HostPlugin(ctx, config)` is the default
  export. Each runtime/UI entry is a self-contained ESM module: no unresolved imports
  or top-level await. Await work inside activation or callbacks.
- The VM provides Host SDK capabilities, not Node APIs. Type-only SDK imports are
  erased when bundling. The SDK workspace is private, not an assumed npm release.
- A contribution, name, Session ID or stored grant ID is not permission. Use the
  context of the actual Agent/Remote/background call and respect retirement.
- Preserve operation IDs and receipts for accepted work. An uncertain response
  requires inspection/recovery; it is not evidence that an effect did not occur.
- Runtime definitions and per-step captures must remain deterministic. Physical
  request retries retain their frozen inputs and implementation binding.
