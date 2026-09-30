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

# State, authority and lifecycle

Read the relevant exact contracts in `references/sdk/host.ts`, `authorization.ts`,
`permissions.ts`, `filesystem.ts`, `http.ts`, `process.ts`, `terminal.ts`,
`credentials.ts` and `clients.ts` under `references/sdk/`.

## Choose the actual authority

| Context | What it provides |
| --- | --- |
| Activation `ctx` | Scoped registrations, package data/storage, preferences, secret vault, explicitly mounted inputs; no implicit Session effect authority |
| Agent `call` | The admitted invocation and ResourceContext capabilities, subject to its captured permission boundary and current lifecycle |
| Remote `caller` | Authenticated client/document identity and read views; independent effects require the authorized callback context |
| `ctx.withAuthorization(id, use)` | Reopens current explicit consent for background/independent work; the ID is not a bearer token |
| TUI UI factory | Pure builders and invocation context; business authority remains in the backend |

Keep domain policy, durable job intent and migrations in the plugin. Host owns
admission, execution identities, credentials, canonical receipts and resource
settlement. A Session ID, absolute path, discovered method or enabled contribution
is only a locator/observation. Native messages into plugin-managed model Sessions
require the owning behavior's explicit nativeInput opt-in.

Use Agent `call.permissions` for necessary extra execution access. Independent
Remote work uses explicit authorization through the current caller; backgrounds
restore that grant. A grant is checked again for new work after restart/revocation.
Directory-only consent does not create a workspace or grant process execution.
Network consent does not imply file writes. Agent calls capture a permission
boundary; changing a Session affects subsequent calls, not already admitted work.

## Store the right things

`ctx.storage` is package/scope namespaced JSON with CAS revisions. Read a key, then
batch mutations against the observed revision; null means never existed, not
currently deleted. Tombstones keep their revision. Up to 128 mutations are atomic,
with at most 1 MiB per value and 16 MiB total data. Scan is ordered pagination,
not a cross-page snapshot. Put domain state and its operation receipt in the same
batch when they must commit together. A conflict means re-read/reconcile, not
silently overwrite a user's changes.

`ctx.credentials` is the package/scope-isolated secret vault with its own CAS.
Keep secrets out of ordinary storage, manifests, instructions, errors and logs.
Use the credential/provider contracts for authentication and revocation; read
non-secret model/preferences discovery separately.

`ctx.data` stores private files and exposes bounded byte/directory operations.
Paths are relative, links rejected and parents must exist. The plugin owns its
file format, migration and crash recovery. `location()` is a display/process-argument
observation, not new filesystem authority. `call.files` performs native typed
read/write/edit/glob/grep/patch operations through Host execution settlement.

`ctx.inputs` exposes explicitly mounted non-secret read-only input selections.
Preparation and capture workspaces expire with those callbacks. Do not retain
these handles for later jobs. Pinned files preserve an opened file identity and
observed length, not an immutable version of in-place changes; validate content
when multiple passes must agree, and close the handle.

## Own all async work

Await resource operations. Close streams, HTTP responses, sockets, process/terminal
handles and execution views using their contract. `ctx.effect(dispose)` registers
reverse-order cleanup; a returned cleanup function belongs to activation as well.
`ctx.run` stages an owned task that starts only after publication is effective.
Use `ctx.sleep` rather than an ambient timer.

`ctx.background.pending(name, wake)` prevents idle expiry while plugin-owned work
remains. It grants no authority and does not prevent explicit shutdown/upgrades.
Persist job intent, restore it after activation, and close the pending registration
when idle. Resume wake callbacks are serial/coalesced, cancellable, and may close
their own registration. `ctx.run` by itself does not keep the Host resident.

Cancellation stops new admission and observation; it does not undo an accepted
external request, file mutation, process action or Session command. Preserve stable
operation IDs. On an unknown outcome, inspect the original receipt and reconcile
with the real system before choosing another mutation. Do not convert unknown
into success, cancellation or a blind retry. Unconfirmed cleanup is different:
it can fence further admission even if a JavaScript catch block handles the error.

## Effects and forwarding

HTTP responses return status normally, including errors, and are streamed via
`next()`; close in finally. Host transport has no automatic redirect/retry policy
for general plugin HTTP; decide retry safety in the owning domain. A closed local
request does not prove the remote server did nothing. Model adapters have a
separate retry/event contract.

Publish a service with `ctx.services.provide(name, invoke)` and acquire it through
`ctx.services.get(name)`, which can return undefined. Declare required service
injections on the composition entry; dependency packages and service injections
solve different readiness problems. Service configuration is a stack of JSON
values interpreted by the provider, not a Node module import or permission grant.

Services forward the original call source and permission boundary. A provider
may receive a discovery context without ResourceContext; inspect the tagged type
instead of inventing Agent authority. Closing a service handle does not cancel
accepted Host work. Notifications require explicit consent and target its client;
delivery uncertainty is not automatically replayable.
