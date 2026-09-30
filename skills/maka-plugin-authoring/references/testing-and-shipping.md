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

# Test, debug, upgrade and ship

## Verify the actual package

1. Validate manifest fields, pinned SDK version and all entry/resource paths.
2. Build each ESM entry and check that it has its default export, no unresolved
   imports or top-level await. `node --check` checks syntax, not Host compatibility.
3. Install into a disposable Host/Profile, activate the intended composition entry,
   and inspect activation status. Exercise the real tool, page, callback or executor.
4. Test the failure paths the feature actually owns: schema/domain rejection,
   permission denial/revocation, cancellation, conflict and unknown receipt;
   retirement/restart if it persists work. Check that cleanup completes and a
   retired registration cannot start new effects.

For a tool, inspect the actual advertised definition and result. For a TUI app,
exercise keyboard and pointer paths, narrow views, localization and dirty drafts;
read-only callbacks must not write. For streams, verify last state, slow consumers,
close and per-reader ownership. For durable workflows, lose a reply and recover
by its original operation ID rather than invoking a second mutation.

In a Maka checkout these are real available commands (scope Rust tests to the
owning crate/test):

```sh
npm ci --no-audit --no-fund
npm --workspace @maka-agent/plugin-sdk run typecheck
node --test packages/plugin-sdk/tests/*.test.mjs
cargo test -p maka-plugins
cargo test -p maka-runtime-host --test integration javascript_plugins
cargo clippy -p maka-plugins -p maka-runtime-host --all-targets -- -D warnings
```

The workspace `justfile` supplies setup, typecheck, test-js and Rust recipes.
Use `CARGO_INCREMENTAL=0` on constrained disks; generated target artifacts are
rebuildable, but do not clean while another build is using them.

## Distinguish common failures

| Symptom | Check |
| --- | --- |
| Package rejected | schemaVersion, supported sdkVersion, missing entry, unresolved imports, invalid/oversized/case-colliding paths |
| Installed but tool/page absent | composition entry/scope/disabled flag, activation error, missing dependency or service, bind returning null, discovery vs description loading |
| Resource call denied | actual call source and current grants, Session boundary, owner retirement; a configured path/grant ID alone is insufficient |
| Code Mode cannot invoke | directOnly/nesting and tool semantics; loading a description does not confer nested permission |
| UI fails while business keeps running | UI factory/builder/byte limit, exact backend forwarding, stable keys; inspect the page failure instead of restarting unrelated activations |
| Submit outcome unknown | stored operation receipt and original revision/input; do not replay based on missing UI acknowledgement |
| Background disappears when idle | durable intent restoration and pending registration; ctx.run alone is not residency |
| Old behavior after editing source | installed immutable package needs update/reinstall; restarting uses the installed bytes |

Preserve a domain error's meaning through the supported error contract. Remote
errors can carry invalid/revoked/cancelled/outcome_unknown/unavailable; unclassified
exceptions become unavailable. Keep provider-specific failures in the provider.
Never include secrets in diagnostic dumps or turn missing usage/outcome into zero.

## Package delivery and upgrades

A package contains at most 256 files, each at most 8 MiB, and 16 MiB decoded total.
The manifest is at most 256 KiB. Ship dist entries and required resources, not the
development dependency tree. Package export produces the Host's installed immutable
bytes; the bundle codec verifies each file and the overall package digest.
Use the native plugin **Export package** workflow or public package export API;
do not fabricate a bundle hash or claim an unpublished registry release exists.

Installation and default composition are distinct operations even when a package
supplies a default patch. Preview the destination package/update. A new installed
generation retires the old owner; callers already admitted must settle. Preserve
plugin-owned storage/schema migrations and durable job/receipt compatibility.
Removing an instance is different from uninstalling its package, and neither is
a substitute for domain deletion of the user's data.

Report the built files, tested SDK/Host version, actual exercised paths, and any
untested remote service behavior. Stop when the requested plugin works and its
necessary failure/recovery behavior is verified.
