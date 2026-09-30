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

# Providers, adapters, executors and orchestration

These are different extension points. Choose the one matching the feature rather
than wrapping every integration in a new model provider. Read the exact shipped
contracts in `references/sdk/providers.ts`, `models.ts`, `llm.ts`, `execution.ts`,
`host.ts`, `history.ts`, `session-import.ts`, `interaction.ts` and `clients.ts`.

## Model provider and protocol adapter

`ctx.modelProviders` registers a provider's identity, catalog/connection workflow
and authentication behavior. Provider-specific validation, diagnostics and errors
belong in that provider. Treat credentials as Host-managed secrets, not ordinary
configuration or global plugin storage. Use current SDK versioned callback shapes;
do not assume an OpenAI-compatible endpoint implies a particular authentication
flow, request field or provider-tool protocol.

`ctx.modelAdapters.register(name, open)` implements a wire protocol. open receives
request/conversation lifetime and returns stream plus optional history confirmation.
Host captures the registration per logical step, resolves credentials and owns
admission, cancellation, usage budgets, retry orchestration and canonical settlement.
The adapter encodes requests, emits typed events with backpressure and classifies
retry safety. Preserve evidence about partial output and tool execution; transport
failure alone does not prove a request is safe to replay.

HTTP bodies expire with their call; sockets belong to the adapter session. A socket
rebind uses the next call's transport. Changed routing identity invalidates cached
connections. Missing/retired adapters fail explicitly; do not silently switch to a
different implementation. Inspect exact model fields and provider capabilities
before mapping context/output limits or server-side tools.

For a small auxiliary generation inside an already authorized call, use
`call.llm.generate` instead of writing a provider. It uses the selected Host model
and transport/OAuth, sends only its explicit prompt/system without inherited
conversation or tools, and settles usage durably. Read its bounds and optional
usage semantics; missing usage is not zero. `ctx.models` is non-secret discovery,
not permission to generate.

## External executor

`ctx.executors.register(definition, execute)` implements an external agent/run
backend. Its request carries committed settings, invocation, conversation key,
content, cwd and instructions. Await `call.emit` for durable output/thinking/tool
observations. Those observations are not executable Host tool dispatches. Return
a typed completed/cancelled/failed outcome. Advertise only implemented capabilities,
including historyCopy if the executor can start from Host-owned copied history.

Executor settings are different from model connections. Configuration changes
apply to later runs; current execution retains the exact executor identity and
complete settings selected at admission. ctx.executors.search discovers scoped
choices, does not grant execution, and requires narrower searches when incomplete.

## Session behavior and input policy

`ctx.behaviors.register(name, prepare, {nativeInput})` supplies preparation for
plugin-managed model Sessions. It can narrow tools, select native workspace versus
attachment-only tools, request client capabilities and add instructions. Preparation
is not an execution grant. Preserve a revision basis when domain changes can make
prepared work stale.

Native user input defaults to denied. `native_user_messages` is an explicit opt-in
for Sessions managed by the same package/scope; it preserves native message,
attachment, queue and receipt handling and does not grant manager/configuration
commands or orchestration overrides. Ordinary Agent selection uses a behavior name;
Plan uses `<name>:plan`. An explicit authorized per-Turn behavior selects its exact
registered name. The selected identity survives continuation.

## Owned executions and history

Open an execution view from the actual call context, close it after use, and choose
the exact command/capability in execution.ts. Root creation and child creation have
different authority. Stable operation IDs are required for Session mutations;
equal retries recover their original receipt, changed inputs conflict. Closing an
observation/view does not cancel accepted Host work.

Persist operation IDs before submitting/enqueueing. Query message receipts to
reconcile pending/cancelled/delivered input, and retract only input not yet delivered.
Package-owned interaction offers can ask questions/forms, not issue permission
approvals. Stopping a wait is not withdrawing an offer, and close cannot replace
an already committed answer.

History catalogs provide metadata, not execution authority. Agent history recall
and independently authorized read_history have different scopes; use the supplied
context rather than a guessed Session ID. Follow returned fences/cursors and keep
unknown/incomplete states explicit. Source messages are original inputs, not
aggregate transcript rows or admission proofs.

Copy/import APIs create historical evidence, not executable calls. Follow their
staging/receipt and publication protocol; accepted copies retain workspace identity
and frozen history membership. Executor-private state is not implicitly copied.
`isolated_git` children use Host-owned linked worktrees and export a settled patch;
export does not merge into the parent. Keep plugin business decisions separate
from Host execution/graph facts.
