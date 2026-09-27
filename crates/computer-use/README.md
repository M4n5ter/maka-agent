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

# Computer Use

Maka owns the Computer Use object API and an isolated, persistent JavaScript REPL.
Cua supplies native platform code, pinned to `5b3d48dfda23bde15ae1f2c150940defbdc64c21`.
See [中文说明](README.zh-CN.md) and [API reference](src/api.md).

The built-in plugin publishes two direct tools, `cua_repl` and `cua_reset`.
They work with Code Mode disabled and are not callable through Code Mode's
nested tool catalog. `cua_repl` accepts `{code, timeout_ms?, title?}`; it supports
real top-level await and persistent `const`/`let` bindings. There is no Node,
filesystem, arbitrary network API or network inspector listener in this VM.

```js
const app = await cua.getApp("Calculator"); // emits initial state
await app.click(7); // index must come from the actual observation
await app.getAXState();
```

`cua.getState()` discovers native applications and configured browser providers.
`getApp`, `getTab` and `createBrowserTab` emit initial accessibility state.
Observations emit by default; `nodeRepl.write` and `nodeRepl.emitImage` provide
explicit output. The bundled `maka-cua` skill explains the workflow and is
available from the native Skill library without automatic installation.

## Ownership and lifecycle

The Host owns one Cua driver (Cua permits one embedded driver per process).
Each Maka Session owns its REPL, native Cua session, opaque target bindings and
observations. Every operation uses fresh Run authority, canonical approval and
its own journal settlement. Explore refuses access; Ask requests a Computer Use
Session grant; Bypass skips that prompt. A retained JS object never retains a grant.
The shared input queue serializes access to the physical keyboard and clipboard.

App bindings retain process start identity; on macOS they also retain the exact
AX window and elements. Accessibility text and diffs are presentation, not an
identity parser. Browser refs bind to backend DOM IDs and a document loader;
browser restarts, navigation and closed tabs cannot silently retarget an object.
Screenshot coordinates are checked against capture dimensions and window/viewport
geometry, then consumed by an action. Screenshot-only observations clear AX refs.

`cua_reset` clears this Session's JS and native bindings without closing user apps
or tabs. Session retirement and Host shutdown drain and release resources.
Cancellation/timeout terminates the REPL and requires reset; already-dispatched
native work still settles before the call returns. Actions are never automatically
replayed. Source, heap, calls, output, targets and live Session counts are bounded.
V8's heap guard is not an OS process memory sandbox.

## Browser connection

Configure the built-in plugin with an existing loopback CDP endpoint:

```json
{"browsers":[{"id":"chrome","endpoint":"http://127.0.0.1:9222"}]}
```

Maka attaches to that browser and its existing profile. It never enables debugging,
restarts a browser, silently changes profiles or accepts an endpoint from model JS.
Missing providers report unavailable. Changing a provider after binding tabs requires
reset. App and browser inventory errors remain distinguishable in `getState()`.

Browser targets support AX/screenshot observation, element and coordinate actions,
text selection, value editing, text/HTML paste, keys, navigation and tab creation/close.
Navigation waits for the selected document to become observable. Readiness does not
mean that every asynchronous application task has completed; verify the visible result.

## Platform boundaries

macOS semantic actions use Cua's bounded AX walker and exact-window checks with
Maka-owned retained elements. `selectText` uses UTF-16 ranges and read-back;
secondary actions must actually be exposed by the element. Clipboard paste retains
all readable formats within 16 MiB, supports UTF-8 HTML, and preserves an intervening
user copy. Paste temporarily activates the exact target, restores its focused field
and selection, and waits for an observable text change before restoring the clipboard.
An unchanged value is reported as uncertain, not fabricated success.

Native screenshot capture, typing, keys, pixel clicks, scrolling and dragging use
Cua. macOS/Linux pixel clicks default to background delivery; Windows input and
native drag may activate the window. There is no automatic foreground fallback.
Native pixel scrolling currently uses page units, not exact pixel distances.
Markdown paste inserts Markdown source as text. macOS name/path/bundle lookup can
launch an app; Linux/Windows require an open window. Native text selection is macOS
only. An unsupported operation fails before sending its input.

The CDP adapter does not implement client tab mentions, extension/IAB providers,
visibility/session-name options, deliverable/handoff UI markers or secondary AX
actions. Its accessibility coverage is the main target; cross-process iframes are
not advertised as complete. Native opaque surfaces without AX have weaker window
lifecycle evidence than retained macOS AX targets. Linux and Windows have not been
accepted on physical desktops in this change. Full cua_repl equivalence or superior
latency/token usage is not claimed.

Perception (`cua-perception`, `cua-som`, `parse_visual_regions`) and model downloads
are absent. This does not require Desktop UI integration, Python or a cloud service.

## Build and verification

macOS needs a working Xcode/Swift toolchain and the calling executable's Accessibility
and Screen Recording grants. Final executables need `-Wl,-rpath,/usr/lib/swift`;
Maka CLI and Host test build scripts supply it. Linux requires X11 development
libraries; compositor-specific Wayland limitations remain explicit.

Hermetic tests cover REPL isolation, fresh call authority, cancellation/settlement,
resource limits and Host approval/lifecycle behavior. Ignored native acceptance tests
in `tests/repl_browser.rs` operate disposable fixtures: a temporary Chrome profile
and a compiled AppKit form, including Unicode editing, rich paste, screenshots,
navigation and stale targets. Set `MAKA_CUA_TEST_CHROME` to override the Chrome binary.
They are separate from ordinary CI tests and do not prove untested desktop behavior.
The form editing test temporarily takes keyboard focus; run it while the user is not
typing. The recovery test stays in the background and uses no keyboard input:

```sh
cargo nextest run -p maka-computer-use --test repl_browser -E 'test(native_observation_recovers)' --run-ignored all
```

See [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES) for source and licensing.
