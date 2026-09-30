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
Cua supplies native platform code, pinned to the [M4n5ter fork](https://github.com/M4n5ter/cua)
at `fdf0267a47f493085accae2a0086739fe4505f17` for the X11 cursor badge fix
([upstream PR](https://github.com/trycua/cua/pull/4260)).
The fork also includes upstream's macOS recording-finalization fix from
[PR #4238](https://github.com/trycua/cua/pull/4238).
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
explicit output. Do not wrap automatically emitted observations in those helpers.
CUA deduplicates identical images within each evaluation before counting output
budget, without retaining an image cache across calls. Inventory errors remain
visible even with `emit:false`. The bundled `maka-cua` skill explains the workflow and is
available from the native Skill library without automatic installation.

## Ownership and lifecycle

The Host owns one Cua driver per desktop process (Cua permits one embedded driver per process).
Each Maka Session owns its REPL, a bound SDK `CuaDriverSession`, opaque target
bindings and observations. Inventory and input share that bound surface; there
is no implicit driver session or per-method session-label injection. Reset creates
a new private lifecycle identity. Driver leases have explicit 24-hour total and
idle bounds; expired authority requires reset and fresh observation, never action
replay. Normal close ends native state before revoking the lease; after expiry the
SDK collects the isolated old native episode through its idle cleanup. Every operation uses fresh Run authority, canonical approval and
its own journal settlement. Explore refuses access; Ask requests a Computer Use
Session grant; Bypass skips that prompt. A retained JS object never retains a grant.
The shared input queue serializes access to the physical keyboard and clipboard.
For logical model steps with callable CUA tools, the plugin supplies a read-only snapshot of REPL
state and configured browser providers. This reads existing memory without starting
a VM or inspecting the desktop. Later tool results supersede a frozen retry snapshot.
A Host restart therefore cannot silently imply that old JavaScript bindings survived.

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

## WSL and desktop selection

The plugin's `desktop` setting defaults to `auto`: Windows in WSL, the local OS
elsewhere. Use `{"desktop":"local"}` to target a Linux desktop inside WSL, or
`{"desktop":"windows"}` to require Windows. Changing the selected desktop requires
`cua_reset`; an unavailable Windows connection never falls back to Linux.

Native releases automatically prepare their Windows component on first use;
no separate Windows installation or PATH setup is needed. Maka downloads its
exact release version through the existing verified native-package cache, then
reuses it offline. Updating Maka selects the matching component automatically.
The first download requires network access and happens before the REPL deadline.
WSL Windows interoperability and an interactive Windows user desktop are required;
a service/SSH Session 0 cannot control another user's desktop.

Maka starts a private Windows helper over inherited stdin/stdout, without a
network port or persistent service. Native targets, process identity, browser
CDP connections and synthetic cursors live in Windows. The Agent, REPL, approvals
and operation journal remain in WSL. `cua.computer.target` reports `windows`.
Browser loopback URLs refer to Windows in this mode.

Sessions share one helper; reset removes only the current Session's bindings and
cursors. EOF stops new work and releases desktop resources after accepted work
settles. A lost acknowledgment is reported as uncertain and never retried; restart
the Host if its desktop connection is lost. Library executables supporting the
helper call `cursor::bootstrap()` and then `desktop::bootstrap()` at process entry;
their Host registers a trusted resolver with `desktop::register_windows_helper`.
Unreleased source builds can use the development override
`MAKA_CUA_WINDOWS_EXECUTABLE`, an absolute WSL path to their own Windows build.

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

## Session cursors

Each Session uses Maka's bare app mark (the A-shaped outline, diamond opening
and detached top bar) as its synthetic cursor, in one of six colors. The whole
mark points upper-left; the leading edge at the center of its top bar indicates
the exact position. There is no enclosing pointer or background tile.
Inputs provide automatic feedback; explicit `app.moveCursor(indexOrPoint)`
and `tab.moveCursor(indexOrPoint)` only indicate a validated target. Their coordinates
use the current screenshot without consuming its mapping.

```js
await cua.cursor.configure({ label: "Research", color: "blue", reducedMotion: true });
await tab.moveCursor(7); // use an index from the current observation
await cua.cursor.getState();
```

The CLI/service registers a private renderer entry point using the same executable.
The renderer inherits only display-related environment, reads a bounded typed stdio
protocol, and reuses Cua's transparent, mouse-through overlay. It has no input operations
or public socket. macOS owns AppKit on that child's main thread. Library hosts call
`cursor::bootstrap()` at process entry or pass a trusted, compatible executable through
`Driver::with_cursor_executable`. Renderer failure leaves input outcomes unchanged;
configuration explicitly permits retry after a failure.
Automatic native feedback uses a bounded display queue and drops stale cues; it
does not wait for rendering before dispatching input. Explicit cursor movement and
state queries await a response. Visibility describes the target's rendered surface,
which may still be covered by another window or an inactive browser tab.

CDP renders the same artwork in an isolated world with a decorative, fixed-position
shadow tree. It does not participate in layout, hit testing or accessibility. Idle
DOM cursors expire and remove themselves, including after a lost connection. Session
retirement removes only its cursor; Host shutdown reaps the native renderer.

Artwork is defined in `src/cursor/theme.rs` and compiled into bounded Cua vector
artifacts in a private temporary directory. Custom model-provided theme code and
arbitrary theme paths are not accepted. Generate a comparison preview with
`cargo run -p maka-computer-use --example cursor-preview -- preview.png`.
Native display availability requires an interactive desktop; the upstream macOS
renderer currently uses the main screen, X11 and Windows use their platform overlays,
and this adapter does not enable a Wayland overlay. Windows native display has been
validated in an interactive desktop Session; Linux X11 checks use a dedicated Xvfb
display. Multiple visual pointers do not create separate OS keyboard or focus seats.

Linux builds need the X11, Xi and Xtst development libraries. Windows MSVC builds
also need the matching Spectre-mitigated CRT libraries. The pinned fork fixes X11
Session labels duplicating across render tiles and glyph seams at tile boundaries.
The regression covers overlapping labels and screen edges; native Xvfb checks with
and without a compositor verify two labels and independent cursor cleanup. These
focused checks do not replace Cua's complete desktop certification matrix.

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
lifecycle evidence than retained macOS AX targets. Windows observation, semantic
text edits, screenshots and cursor lifecycle have been verified from WSL against
a disposable native form. Linux native interaction coverage remains incomplete.
Full cua_repl equivalence or superior latency/token usage is not claimed.

Perception (`cua-perception`, `cua-som`, `parse_visual_regions`) and model downloads
are absent. This requires neither Python nor a cloud service.

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

For native cursor acceptance, build `maka-cli --bin maka`, set `MAKA_CUA_TEST_HOST`
to that executable, and select `test(native_cursors)` in the same test target. The test
captures the renderer's own window and checks multiple colors, Session cleanup,
unchanged system pointer/focus and process retirement without keyboard input.

See [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES) for source and licensing.
