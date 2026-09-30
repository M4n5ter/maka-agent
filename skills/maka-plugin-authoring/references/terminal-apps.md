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

# Declarative terminal apps

Read `references/sdk/terminal-view.ts`, `terminal-app.ts` and
`terminal-transcript.ts` for the complete View 9 builders, fields, actions, backend
requests, receipts and transcript protocols. `assets/starter/` is a runnable
read-only page with a separate UI module.

## Business activation and UI factory

```js
await ctx.tui.app('overview', {
  entry: 'panel.mjs',
  async backend(request, cx) {
    if (request.kind === 'read') return {message: 'Ready'};
    return {kind: 'rejected', message: 'This page is read-only'};
  },
}, {title: {fallback: 'Overview'}, context: 'application'});
```

```js
// panel.mjs — its own self-contained ESM entry
export default ({tui}) => ({
  async read(_route, cx) {
    const model = await cx.backend();
    return {title: 'Overview', revision: '1', root: tui.text('message', model.message)};
  },
  submit: (_submission, cx) => cx.backend(),
});
```

The backend remains in the business activation. A UI factory is initialized once
per stable document and gets pure builders. Invocation context contains locale,
t(en, zhCN, zhTW), signal and a parameterless backend(). It can forward the exact
original request once per invocation, not arbitrary inputs or another app's method.
There is no backend authority during factory initialization. The SDK supplies the
view version; use builders rather than fabricating a parallel terminal protocol.

Placements: page (default), settings (application context), panel/status (Session
context), or slot(name). Page descriptors alone may declare slash commands.
Session-inspector slots receive observation such as Session ID and locale, not
execution permission. A slot's context should hold stable entity IDs and its node
key should distinguish independently editable entities.

## Layout and editing

The terminal owns focus, scrolling, selection, local typing and drafts. Use semantic
tones and stable sibling keys without `/`. A selected/current row and keyboard
focus are separate states. Never emit terminal escape codes or manually invent
mouse geometry. Keep actions/fields declared in the returned view consistent with
controls that reference them. Use the locale helper for user-visible text.

Rows/columns, Markdown/code, tabs, fields, split panes, boundaries and collections
are in the builder contracts. Fields/buttons must remain separately reachable
outside a list's arrow-navigation group. Local filtering, split resizing, collection
highlighting and drag previews make no backend mutation. Collections submit the
chosen stable item/group/before values through one declared action and the normal
revision contract.

A view has a 64 KiB encoded budget. Put large history in transcript resources,
not thousands of Markdown children. Page VMs have a 128 MiB JS heap budget and
200 ms synchronous slice; this is not total RSS or a security sandbox. A page
failure preserves its last valid view/transcript for local reading, without
restarting the business activation or sibling page VMs.

## Writes and uncertain outcomes

The backend enforces domain authorization and compares request.revision with its
stored revision. Declare recovery only when a durable operation receipt can be
looked up. Commit a state change and its receipt together. UI code forwards
submit/recover to cx.backend(); it cannot fabricate a successful receipt.

`{kind: 'applied', route}` completes the form and opens the returned route.
`{kind: 'updated'}` is a Submit-only outcome for accepted actions that must retain
the same document/streams (for example starting a sign-in flow). It rereads the
route, resets submitted fields and preserves unrelated drafts. It confirms the
action, not an eventual background result. Recovery uses a durable applied receipt,
not updated. A conflict or unknown outcome remains explicit; avoid automatic replay.

## Live changes and history

`const changed = await ctx.tui.changes('changed')` creates a coalesced invalidation
stream; use `changes: 'changed'` in the descriptor and notify after commit. Each
document owns its subscription, while the business producer may serve many pages.
Dirty drafts survive invalidation and use the normal conflict review.

`ctx.tui.transcriptResource(name, {blocks, timings?})` provides native Chat's paging,
Markdown, grouping, selection and search. Register its resource in the app's
`resources` declaration, include it in the backend model, and render
`tui.transcript(key, model.resource)`. Close the store with activation cleanup.
Stable `{turn,message,part}` keys plus revisions identify records; append/replace/
remove/timing update content without rereading the entire view. Mounts, snapshots,
cursors and cancellation belong to the resource helper, not a second UI cache.
The SDK documents retained-source/page/fragment limits and reader invalidation.

Source-checkout examples: `crates/cli/tests/fixtures/board-plugin/` exercises CAS,
live transcript and collection editing; `memory-plugin/`, `faults-plugin/`,
`nested-navigation-plugin/` and `configured-plugin/` exercise lifecycle boundaries.
Their real PTY tests live in `crates/cli/tests/integration/tui/`.
