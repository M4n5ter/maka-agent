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

<h1 align="center"><img src="assets/logo.png" alt="Maka" width="72" /> Apache Maka (Incubating)</h1>

[简体中文](README.zh-CN.md)

Apache Maka (Incubating) is a high-performance agent workspace that keeps a complete record of everything it did.

Maka runs agents through a native Rust CLI and TUI. A Runtime Host owns execution, permissions and durable state. Model providers, tools and business capabilities use public plugin interfaces.

## Run from source

Install Rust 1.98 or newer, Node.js 24 LTS, npm 11.19.0 and [just](https://github.com/casey/just). Native Computer Use additionally needs Xcode Command Line Tools on macOS or the X11 development libraries on Linux.

```sh
git clone https://github.com/apache/maka.git
cd maka
just setup
just run
```

`just run --help` lists the CLI commands. `just build --release` produces `target/release/maka` (`maka.exe` on Windows). Node.js bundles provider SDKs at build time; the resulting executable runs its embedded JavaScript in V8.

## Repository

| Directory | Purpose |
| --- | --- |
| `crates/` | Runtime Host, CLI/TUI, plugins and native platform capabilities |
| `packages/plugin-sdk/` | Host and declarative terminal-view contracts for JavaScript plugins |
| `packages/cli/` | npm launcher for platform-specific Rust executables |
| `scripts/rust/` | Embedded SDK bundling and source-bound npm packaging |
| `website/` | Independently built project website |

Use `just --list` for development commands. See [Contributing](CONTRIBUTING.md), [Architecture](ARCHITECTURE.md), [Documentation](docs/README.md) and [Security](SECURITY.md).

## Distribution

The npm launcher selects a matching native package; it contains no agent runtime. Native preview artifacts are built and verified from one source archive. Packaging and publication are documented in [packages/cli](packages/cli/README.md).

Apache Maka is undergoing incubation at the Apache Software Foundation. See [DISCLAIMER-WIP](DISCLAIMER-WIP), [LICENSE](LICENSE) and [NOTICE](NOTICE).
