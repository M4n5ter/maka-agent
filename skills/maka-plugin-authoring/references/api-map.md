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

# Public API map

The bundled skill ships the complete current public SDK source, byte-for-byte from
`packages/plugin-sdk/src/`. Read it using `Skill` resources; do not infer a method
from a similarly named API in another agent framework. Source imports keep their
sibling `.js` module specifiers; TypeScript resolves these to the `.ts` contracts.
All files in the table are available under **references/sdk/** after installation.
In a Maka source checkout they live in **packages/plugin-sdk/src/**; they are
embedded at build time rather than checked in as a second copy here.

```json
{"name":"maka-plugin-authoring","resource":{"path":"references/sdk/host.ts"}}
```

Follow the complete `page.next` Skill input for remaining content. Use line ranges
for a known interface. Main entry Host SDK version is 3; terminal View version is 9.
The exact types/comments in these files are authoritative for parameter names,
optionality, unions, return shapes, budgets and cancellation behavior.

| File | Read for |
| --- | --- |
| `host.ts` | HostPlugin/HostContext; lifecycle, tools and bindings, prompts, services, storage, behaviors and input preparation |
| `authorization.ts` | Explicit consent requests, capabilities, grants and resolved boundaries |
| `permissions.ts` | Agent-call permission additions and their execution boundary |
| `credentials.ts` | Package/scope secret vault, revisions and deletions |
| `filesystem.ts` | Read-only directories/pinned files, typed native file operations and byte entries |
| `database.ts` | Bounded trusted-path read-only SQLite queries and typed cells |
| `http.ts` | Scoped HTTP requests, streamed responses and cleanup |
| `process.ts` | Process startup/control/output and durable effect outcomes |
| `terminal.ts` | PTY/terminal control, screen/output observations and lifecycle |
| `clients.ts` | Authorized client capabilities, tool calls and notifications |
| `execution.ts` | Session configurations, owned roots/children, submission, queries, queues, worktrees and execution views |
| `interaction.ts` | Package-owned questions/forms, offers and answers |
| `history.ts` | Session catalogs, recall reads, sources and owned history copies |
| `session-import.ts` | Historical import staging, records, receipts and publication |
| `llm.ts` | Model discovery/selection and auxiliary generation |
| `models.ts` | Protocol adapters, request/response events, transports, session lifetime and retry evidence |
| `providers.ts` | Provider definitions, connection/catalog/authentication workflows and diagnostics |
| `input-resources.ts` | Composer resource search, preview, selected identities and pagination |
| `usage.ts` | Physical attempt activity, cursor fences, refinements and summaries |
| `pricing.ts` | Rate catalog, update CAS and explicit pricing consent |
| `terminal-view.ts` | View 9 nodes, placements, fields, actions, requests and resource declarations |
| `terminal-app.ts` | Pure view builders/factory contexts, backend forwarding and receipts |
| `terminal-transcript.ts` | Transcript records, resource protocol, store helpers, snapshots and readers |

## Read-oriented sources and ownership

- `ctx.inputs` and preparation/capture workspaces are bounded directory views;
  callback-local views expire with the callback. They never imply writes/processes.
- `caller.views.session/workspace/projects/queryDatabase` follow authenticated
  Remote scope and path-access requirements. Inspect types before accepting raw paths.
- Usage/pricing/Session metadata catalogs are observations. Pagination completeness,
  missing usage, settlement fences and revision changes are meaningful states.
- Model/tool capture receives frozen facts; later effects use their actual call.

For Rust trait owners and in-repository consumers, read `references/native-rust.md`.
For wire package management use the native editor or an authenticated client with
`crates/protocol/src/plugin/` contracts in a source checkout. Package operation
names are not SDK methods or standalone CLI subcommands.
