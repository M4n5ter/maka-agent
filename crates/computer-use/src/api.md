Computer Use is an independent persistent JavaScript REPL. Use cua_repl directly;
Code Mode is not required. Variables and bound objects survive calls and Turns within
the current Host process. A fresh REPL after restart has no variables from history;
follow the current Computer Use state supplied with the model request.
`cua.computer.target` identifies the selected desktop platform (`mac`, `linux`, `windows`).
In WSL it defaults to the Windows host; shell commands may still run in Linux.

For a fresh REPL, execute exactly one entry point first, then read its documentation
and initial state before choosing further calls. Browser IDs come only from
configured providers; when none are configured, use native browser windows through
getApp. The API does not expose driver administration such as start_session.

Entry points (selection emits initial state):
- `await cua.getState({emit?})`: app and configured browser inventory, including inventory errors.
- `await cua.listApps({emit?})`, `await cua.listWindows({emit?})`.
- `const app = await cua.getApp("Name / bundle ID / app path")` or `getApp({windowId})`.
  Select an exact window when names are ambiguous. macOS may launch the app;
  Linux and Windows require it to be open already.
- `await cua.listBrowsers({emit?})`, `await cua.listTabs({browser?, emit?})`.
- `const browser = await cua.getBrowser({id? , url?})`: select without opening a tab.
- `const tab = await cua.getTab("tab ID", {browser?})` or `getTab({url}, {browser?})`.
- `await cua.createBrowserTab(browserId, url?)`: open in the configured profile.
  Unavailable providers, tab mentions and unsupported creation options throw.

App and Tab observations:
- `getAXState({emit?, disableDiffing?})` returns text, using a diff by default.
- `getScreenshot({emit?})` returns Uint8Array and emits the image.
- `getAXStateAndScreenshot(options?)` returns `{state, screenshot}` and emits both.
Discovery, selection and observations emit automatically. Call them directly;
wrapping `getScreenshot()` in `nodeRepl.emitImage()` or `getAXState()` in
`nodeRepl.write()` is redundant. Use `{emit:false}` when processing the result
yourself; inventory errors still appear. Identical images emitted within one call
are delivered once. A later call may emit the same image again.
Screenshot-only observation clears accessibility indices.
If a native AX observation fails while an app is busy, refresh the same bound object
when the app responds again. Failed observations invalidate old indices and coordinates.

App and Tab actions:
- `moveCursor(index | [x,y])` indicates an observed location using a synthetic cursor.
  Coordinates use the same current screenshot as `click`; this does not consume it.
  The returned visibility is display feedback, not evidence of an input effect.
- `click(index | [x,y], {mouseButton?, clickCount?})`, `drag([x,y], [x,y])`.
- `scroll(index | [x,y], "up" | "down" | "left" | "right", pages?)`.
- `setValue(index, string)`.
- `selectText(index, text, {prefix?, suffix?, selectionType?})` where selectionType is
  `text`, `cursor_before` or `cursor_after`. Native selection is macOS-only.
  `text_not_found` means the text or supplied context is absent; read the current
  state again. `ambiguous_text` requires a more specific prefix/suffix.
- `performSecondaryAction(index, action)`: use an exact action listed in the native
  AX observation. CDP does not expose secondary actions and rejects this method.

App input: `typeText(text)`, `paste(text, {format?})`, `pressKey(key)`.
Tab input: `typeText(index|null, text)`, `paste(index|null, text, {format?})`,
`pressKey(index|null, key)`. A supplied index focuses the observed field first.
Use `null` for current focus. Key examples: `Return`, `Tab`, `super+a`, `shift+Left`.
Paste formats: `text` (default), `md` (Markdown source as plain text), and `html`.
Native HTML paste is macOS-only and needs a readable editable value and selection. It temporarily
activates the exact window, preserves its selection, and restores the prior clipboard
unless the user copied something newer. An unchanged value is reported as uncertain.
Browser HTML paste requires an observed contenteditable element.

Tab navigation: `goto(url)`, `back()`, `forward()`, `reload()`, `close()`.
Navigation supports HTTP(S) and about:blank and waits for an observable document.
The main CDP target is covered; cross-process iframe accessibility is not complete.
Native scrolling accepts page counts; exact pixel distances are currently unsupported.
CDP scrolling also accepts `{pixels:500}`. Native indexed clicks support one semantic
click; use screenshot coordinates for multiple clicks when needed.

After deterministic actions, fetch `getAXState()` in the same call before deciding
what to do next. Do not guess delays before observing. A successful input dispatch
is not proof of the user's requested outcome: verify the resulting state.
Coordinates belong to the latest screenshot. Moving/resizing, navigation, scrolling
or any action consumes/invalidates the mapping; capture again before another point.
macOS/Linux pixel clicks default to background delivery; Windows input and native
drag may activate the target. No automatic foreground fallback occurs.

`nodeRepl.write(value)` emits text/JSON. `await nodeRepl.emitImage(bytes | dataUrl |
{bytes, mimeType})` emits an image. File/HTTP image URLs, Node, arbitrary filesystem
and network APIs are unavailable. `cua.rewriteDocumentation()` reprints this reference.

Every operation uses current Host permissions; JS objects retain no grants.
In Ask mode, Computer Use approval happens before the REPL execution timeout starts.
Native macOS operations report `desktop_locked` while the screen is locked; connected
CDP providers and pure REPL computations remain independently available.
Each Session has its own Maka cursor. Input actions show it automatically; call
`moveCursor` only to point without acting. `cua.cursor.configure({label?, color?,
enabled?, reducedMotion?})` changes this Session's display. Colors are `blue`, `mint`,
`violet`, `amber`, `rose`, `cyan`; labels allow up to 48 characters. `cua.cursor.getState()`
reports settings and native/browser visibility. A native renderer in `idle` state starts
on the first native indication; `unavailable` reports a missing/failed display host.
Configure again to retry a failed renderer. Idle cursors disappear after a few seconds.
Cursors share no input authority and do not move the system pointer. Multiple cursors
do not isolate the physical keyboard, application focus or input method.
Cancelled, timed-out or heap-exhausted REPLs require `cua_reset`, then target rebinding. Admitted
native work settles before returning. Never automatically replay an uncertain action.
Reset clears only this Session's REPL and bindings, leaving user apps and tabs open.
