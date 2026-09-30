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

# Maka experimental Rust preview

[简体中文](README.zh-CN.md)

**This npm distribution is an independent, experimental preview. It is not an official release of the Apache Software Foundation (ASF), and is not approved, endorsed, or supported by the ASF. Its publication does not imply ASF affiliation or sponsorship.** The source repository and Apache-2.0 license do not change this distribution's unofficial status.

Install the experimental CLI explicitly:

```sh
npm install --global @maka-agent/cli@rust-preview
```

`@maka-agent/cli@latest`, the native components’ `latest` tags, and the separate `maka-agent@latest` are empty placeholders with no CLI, runtime, dependencies, or install scripts. The `@maka-agent/cli-*` packages are platform components of the same unofficial preview. Preview behavior and compatibility can change without notice.

The `@maka-agent/cli` npm launcher starts the native Maka CLI for the current OS and CPU. Exact-version optional dependencies carry the executables. Arguments, standard input/output and exit status pass through to the native process.

The supported preview set is macOS arm64, Linux x64 with glibc, and Windows x64. WSL uses the Linux executable and can obtain the matching Windows helper through the native distribution layer.

## Build and publish

Create a committed source candidate with `just source <version>`. Each platform runs:

```sh
just package <source.tar.gz> <target> <build-id> native-preview
```

The package command writes the platform archive and `<target>.json` receipt. Collect all three platforms and the source archive with its SHA-512 sidecar, then run:

```sh
just release-prepare native-preview
just release-check native-preview
just publish native-preview
```

`release-prepare` assembles and validates all four packages from one source. `release-check` installs the launcher and native package offline on the current platform and exercises the CLI/V8; run it on all three platforms. Publication installs the Windows component first, the other native packages next, and the launcher last, then verifies every `rust-preview` tag. Retry the same artifacts after interruption; never rebuild an already published version. Use one publisher at a time per channel: npm tags have no conditional-write API.

GitHub uses the registered `npm-publication.yml` entry. `gh workflow run npm-publication.yml --ref <branch> -f publish=false` produces the complete artifacts and three-platform verification. Unofficial publication runs in the personal fork `M4n5ter/maka-agent`, not the ASF repository. It requires explicit `publish=true` on `feat/runtime-host-rust` and the dedicated `npm-rust-preview` environment. That environment permits only this branch and holds its npm publishing token; the main-branch publication environment is independent. Use `gh workflow run npm-publication.yml --repo M4n5ter/maka-agent --ref feat/runtime-host-rust -f publish=true` to build, verify, and publish the preview.

The shared `latest` placeholder template is maintained in `packages/cli/placeholder/`. Stage a copy with the intended package name, inspect its tarball, then publish it with `--tag latest`; it has no executable code. Never move `latest` to a preview build.

Release packages use pinned V8 builds with optional LGPL glibc math disabled. Native macOS packaging requires Xcode 26 or newer for Computer Use; CI builds on macOS 26 and checks installation on macOS 15. Linux needs glib development files and cargo-zigbuild/Zig. A fallback V8 source build additionally requires Python 3, Git, Clang/libclang 19 or newer and a native C++ toolchain. It downloads Chromium tools, can take over an hour and needs tens of GiB of temporary space, removed after the build.

Completed V8 libraries, bindings and notices are reused from `target/v8` (or `$CARGO_TARGET_DIR/v8`). On a cache miss, packaging downloads the matching bundle pinned in `scripts/rust/v8-prebuilts.json`; only an absent pin triggers source compilation. The key binds V8 source, target, features and producer code, independently of Maka/deno_core versions and download tooling. Downloads must match the source-pinned SHA-256, and every use validates the bundle's file digests. A broken pin or corrupt cache fails explicitly. Native link and smoke checks still run for every package.

For keys without pins, native CI exports `v8-prebuilt-*` artifacts with archives, digests and the validating commit/run. After the complete workflow passes, publish those exact archives as versioned GitHub dependency prerelease assets under a `deps-v8-*` tag and commit their URLs, hashes and validation records to `v8-prebuilts.json`. Never overwrite an existing asset or change the meaning of a key. Actions caches accelerate unpublished builds; the committed manifest is the trust root for downloads. Remove a corrupt local entry (and its Actions cache, if applicable) before retrying; deleting local cache alone normally downloads the pinned bundle again.

After dependency changes, run `node scripts/rust/notices.mjs generate` and review the snapshot. It collects full and nested license texts from pinned dependency sources; reviewed exceptions for missing upstream license files live in `notices-sources.json`. Release builds validate the snapshot's source-input binding and append notices from the actual V8 build graph and Rust standard library. The license TSV is an inventory, not a substitute for these texts.

No install script downloads or executes an unverified helper. Runtime behavior lives in `crates/cli`.
