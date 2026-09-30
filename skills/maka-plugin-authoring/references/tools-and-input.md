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

# Tools, prompts and input preparation

Exact contracts: read `references/sdk/host.ts` for ToolDefinition, tools, prompt,
behaviors and input; `references/sdk/input-resources.ts` for composer discovery.

## Model tools

```js
await ctx.tools.register({
  name: 'ExampleEcho',
  description: 'Return the supplied text unchanged.',
  inputSchema: {
    type: 'object', properties: {text: {type: 'string'}},
    required: ['text'], additionalProperties: false,
  },
  semantics: 'parallel',
}, ({text}, call) => {
  call.signal.throwIfAborted();
  return {text};
});
```

Tool names must be stable and avoid collisions. The input schema is the model's
contract; validate domain values too. Return JSON. Use call.files for typed file
operations, call.http for transport, call.clients for granted client tools, and
other scoped services from ResourceContext. Returned tool text is data; plugin
output cannot grant more capability.

Definitions may specify `outputSchema`, `alwaysVisible`, `directOnly`, `semantics`
and a provider descriptor. Ordinary tools are discoverable through `tool_search`.
`alwaysVisible` includes a tool without search; reserve it for a tool that needs
that visibility. `directOnly` excludes it from nested Code Mode invocation.
Loading a description does not grant permission. Direct/Code Mode visibility and
nested callability are distinct; use declared semantics rather than serializing
all tools in a new plugin-wide mutex.

`parallel` permits concurrent effects; the implementation must support its own
concurrency. `exclusive_step` is exclusive in a step. `finish_turn` declares a
terminal tool. Choose semantics to reflect actual effects, not presentation.
Keep names, schemas and ordering stable across turns when capabilities are unchanged.

## Per-step bindings

Use `ctx.tools.bind(definitions, capture)` when a group needs one frozen handler,
model-dependent availability or supporting context. Capture sees invocation,
behavior, cwd, available tool names, model capabilities and a read-only workspace.
Return `{invoke, context?, providerTools?}`, or null to omit the group. Capture is
observation, not execution: read-only workspace handles expire when it returns.
Tools returned in that model step use the captured closure, including physical
retries. Keep current side-effect authority in the later `call` argument.

`providerTools` maps only this group's declared names to provider-executed
identifiers/arguments. A provider-only binding can omit invoke. Check the model's
advertised provider-tool protocol; these tools run in the primary model request,
not through the local handler. Mixed unsupported capabilities should be absent,
not silently emulated with another provider's protocol or errors.

Closing a registration withdraws that exact generation. It cannot revoke a newer
replacement. Await registration calls during activation so the batch publishes
atomically; a half-failed activation must not appear successful.

## Prompts

`ctx.prompt.section`, `.variable`, and `.context` register prompt contributions.
Sections/context default to templates; `format: 'plain'` is for already resolved
or user-authored text. Text callbacks receive a tagged Session or model-step
request and read-only workspace, not tool authority. A complete section replaces
other prompt contributions; explicit Session/child instructions remain intact.
Dynamic observations belong in context, not unpredictable changes to tool schemas.

## Prepared input and slash resources

`ctx.input.prepare(name, callback, options?)` runs before admission. It receives
original content, selected resources, prior providers' receipts, workspace, tools
and cancellation. Return `unchanged`, `ready` with content and receipt, or `blocked`
with message and receipt. Keep preparation read-only. Accepted content and its
receipt freeze together; later code must not re-read different input implicitly.

For mutable domain state, capture a `ctx.revision()` basis before reading it and
return the basis with preparation. Order domain writes through that revision's
`invalidate(update)`; it controls admission consistency, not your storage transaction.
Close/re-register a provider when its source changes to retire stale preparations.

The resources option exposes bounded, read-only selector discovery to the composer.
Selections are canonical source identities; display labels are not execution
permission. Read the exact InputResources search/preview and pagination types
before implementing a resource picker. UI slash commands are different: a page
app's `commands` metadata merely opens a route, and invokes no business callback
while the user is typing.
