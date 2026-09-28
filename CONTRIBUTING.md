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

# Contributing to Maka

[![docs](https://img.shields.io/badge/docs-%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-blue?logo=googletranslate&logoColor=white)](./CONTRIBUTING.zh-CN.md)

## Where to start

Bug fixes, model provider support, tests, performance work, and documentation merge most readily. Pick something up from [`help wanted`](https://github.com/apache/maka/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22) · [`good first issue`](https://github.com/apache/maka/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) · [`bug`](https://github.com/apache/maka/issues?q=is%3Aissue+is%3Aopen+label%3Abug) · [`enhancement`](https://github.com/apache/maka/issues?q=is%3Aissue+is%3Aopen+label%3Aenhancement) and claim it in a comment. Use the **Bug report** or **Feature request** template for new issues; report security problems through [SECURITY.md](./SECURITY.md), never as a public issue. Questions, ideas, and not-yet-actionable proposals belong in [Discussions](https://github.com/apache/maka/discussions), which reaches the whole team by email.

To self-assign an unclaimed issue, post a comment whose entire body is exactly `take`; post `untake` to remove your assignment. Other claim text does not trigger the workflow.

Project direction, governance, and material product decisions are discussed publicly on [`dev@maka.apache.org`](https://lists.apache.org/list.html?dev@maka.apache.org) before implementation; implementation-level decisions may live in the pull request.

## Human ownership and AI attribution

Every contribution has a human contributor of record who reviews the work, decides to submit it, and owns its accuracy, provenance, and licensing. Agents may commit and push freely; the final review and merge decision always belongs to a human.

Each pull request states whether generative tooling contributed substantively, naming the tool if so; translation, wording edits, autocomplete, and spelling correction don't count. Automated messages must identify themselves. When AI authors a material part of a contribution, add a `Generated-by: <tool>` trailer to each affected commit, and keep it in the final commit through squash or amend.

## Review

Every pull request to `main` needs an approving review from a committer other than the author and a passing `test` check; branch protection in [`.asf.yaml`](./.asf.yaml) enforces both. An approval stays valid across later pushes — push follow-up commits in the open, and ask for another look when the change grows past what was reviewed. The review must be an independent human judgment — AI review does not count. A maintainer decides whether a change is material and whether the review it received is enough.

## Provenance and licensing

Submit only work you have the right to contribute, and record third-party sources, licenses, and attribution. Contributions are licensed under the [Apache License 2.0](./LICENSE); for material AI-generated content, follow the [ASF Generative Tooling Guidance](https://www.apache.org/legal/generative-tooling.html).

## Development

Use Rust 1.98 or newer, Node.js 22.19 or newer, npm 11.19.0, just, cargo-nextest and Python 3. The license inventory check also uses cargo-deny.

Use [just](https://github.com/casey/just) for common tasks. Install it with your package manager (`brew install just` on macOS), or `cargo install --locked just`. On Windows, put Git for Windows' `sh` on `PATH`.

```sh
just setup                 # build-time JavaScript dependencies
just run                   # native TUI
just run --help            # CLI commands
just check                 # formatting, Clippy, SDK, scripts and Rust tests
```

`just test -p maka-runtime-host` runs one crate's tests. Add build targets or release profiles through `just build` arguments. Use the existing Cargo cache and avoid redundant build profiles.

Development builds use Cargo's V8 prebuilts. `just package` reuses our pinned V8 build without LGPL glibc math; source compilation is needed only when no matching prebuilt or local cache exists. See [CLI packaging](packages/cli/README.md) for build requirements.

The website is independent: `just website-setup`, then `just website-dev` or `just website-check`. Its dependency changes update `website/package-lock.json`; build tooling and plugin dependencies use the root lockfile.

Architecture is documented in [ARCHITECTURE.md](ARCHITECTURE.md). Each crate owns its implementation and behavioral tests.

## Rust workspace

- Use `name.rs` with `name/` for children, not `mod.rs`. Keep unit tests at the end
  of their implementation file; integration tests belong in `tests/`.
- Test durable behavior and meaningful failure boundaries. Prefer extending an
  existing test over duplicate fixtures, source-text assertions or real-time sleeps.
- Keep business plugins on public plugin capabilities. Host owns admission,
  canonical facts and resource settlement; plugins own domain policy and data.
- Model closed operations and states with types; generate schemas with `schemars`.
  Database schema changes belong in SQLx migrations, not startup DDL.
- Keep protocol codes and diagnostics separate from localized UI copy. Plugin
  clients use typed `en`, `zh-CN`, `zh-TW` catalogs and preserve the full locale.
  New crate documentation should include concise English and Chinese versions.
- Run `cargo fmt --all -- --check`, strict Clippy, relevant `cargo nextest` tests
  and plugin SDK type checks. Use Biome for TypeScript, including plugin clients.

## Pull requests

Opening a pull request pre-fills [`pull_request_template.md`](./.github/pull_request_template.md); fill it in rather than replacing it.

Branches and titles follow [Conventional Commits](https://www.conventionalcommits.org/): branches are `<type>/<description>`, titles are `<type>(<scope>): <summary>`. The repository squash-merges, so the title becomes the commit on `main`; `git log` shows the types and scopes in use.

For UI changes, include before/after screenshots or a recording. Keep the description short and your own — if it needs many paragraphs, the pull request is probably too large.
