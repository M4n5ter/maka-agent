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

# Maka TUI

[简体中文](README.zh-CN.md)

Terminal client for the Runtime Host, launched by `maka tui`.

The sidebar groups sessions by workspace. **Needs you** filters that directory without leaving the current conversation. `Ctrl+B` opens navigation in a sheet on narrow terminals or in focus mode; `Esc` returns to the current draft. Tab crosses lists and fields, while arrow keys move within a list. `F1` opens help over the current page; Host connection details live in Settings.

`Enter` sends a message, or queues it for the next turn while the model is working. `Shift+Enter` inserts a new line on terminals with enhanced keyboard support; `Ctrl+J` works on legacy terminals, including Windows Terminal through WSL. Pasting multiple lines never sends them. `Ctrl+K` opens commands, `Ctrl+F` searches the conversation, and `Ctrl+N` creates a session. Dialogs and search keep their own input scope.

The sidebar's Search opens the command palette. It scans the Host's paged session directory by name, workspace, ID and displayed labels; **More matching sessions** continues a broad search. Model choosers search enabled chat models across catalog pages.

When the Host has no model connection or default model, the workspace home offers the missing setup step; `Ctrl+N` opens that step. Existing sessions remain available.

Chat and plugin transcripts share Markdown, selection, search and streaming presentation. New streamed text briefly fades to its final color without delaying the received content or changing its layout. Reduced motion and terminal-owned colors display it immediately.

For a selection longer than the viewport, click its start, scroll, then
Alt+click its end (Option+click on macOS). Normal clicks clear the selection;
Shift+arrows and Shift+PageUp/PageDown also extend it. Shift+click uses the same
selection when the terminal forwards it. On Unix, Maka requests `XTSHIFTESCAPE`
while running and releases it on exit, but some terminals reserve Shift for
their own selection regardless. That terminal selection can include sidebars
and cannot be cleared by Maka. Alt+click avoids that Shift policy. In Otty 1.5.4,
`mouse-shift-to-select = false` is an optional global preference for forwarding
Shift too; it is not required for Alt+click.

Settings → Plugins manages local packages, instances, configuration and service bindings. Changes are reviewed before submission; activation updates arrive automatically. Configuration drafts remain in memory, and an uncertain result is never retried automatically.

A session's **Skill library** imports local Markdown, installs sources, reviews managed updates and confirms directory deletion. WorkHub configures new tasks and repairs unavailable models from its views; its coordinator accepts ordinary chat input. Other managed sessions enable native input only when their owning behavior explicitly supports it. Rust and JavaScript plugins can contribute pages, settings, panels and nested views through the same public components.

- `maka-client` owns transport and protocol validation; the TUI owns presentation and input.
- Provider setup consumes public descriptors, configuration and authentication contracts.
- Local state stores drafts and recovery identities, never authentication input. Uncertain writes require observation, not automatic replay.
- Fluent catalogs cover English, Simplified Chinese and Traditional Chinese.
