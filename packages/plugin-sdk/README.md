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

# Maka plugin SDK

[简体中文](README.zh-CN.md)

Public TypeScript contracts for the Rust Runtime Host. **Host SDK 3** and
**terminal View 9** are independent of Maka's application version and this npm
workspace's version. The workspace is private and is not an assumed registry package.

Plugin development has one maintained guide: the
[Maka plugin authoring skill](../../skills/maka-plugin-authoring/SKILL.md).
It is bundled with Maka, including executable starter files and the complete SDK
source contracts. It is installed automatically for each Profile and its complete directory is
replaced with the shipped version at startup, removing obsolete resources. Enable/disable
and pinning preferences are preserved; keep custom changes in a separate skill. Load
`/skill:maka-plugin-authoring` or let the model discover it. `Skill` reads its linked
resources without exposing Host-private directories to ordinary file tools.

Start with the [quickstart](../../skills/maka-plugin-authoring/references/quickstart.md)
and [runnable package](../../skills/maka-plugin-authoring/assets/starter/).
The [API map](../../skills/maka-plugin-authoring/references/api-map.md) routes to
exact [SDK sources](src/host.ts); separate guides cover tools/input, authority/state,
terminal apps, providers/executors, native Rust, and testing/shipping.

```sh
npm --workspace @maka-agent/plugin-sdk run build
npm --workspace @maka-agent/plugin-sdk run typecheck
node --test packages/plugin-sdk/tests/*.test.mjs
```

Package runtime/UI entries are self-contained ESM with no unresolved imports or
top-level await. Use type-only SDK imports during development and the issued Host
capabilities for runtime effects. See the guide for manifests, lifecycle and activation.
