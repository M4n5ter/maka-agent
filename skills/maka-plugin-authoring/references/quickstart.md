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

# Start an installable plugin

The three separate versions are package manifest schema **1**, Host SDK **3**, and
terminal View **9**. Maka's application/npm version does not select the SDK API.
A package is immutable installed bytes; an instance is an entry in the composition
tree; an activation is a live generation of that entry. Installing files alone need
not produce a running instance.

## Runnable starter

Read/copy these skill resources into an empty project directory:

- `assets/starter/maka.extension.json`
- `assets/starter/composition.json`
- `assets/starter/host.mjs`
- `assets/starter/panel.mjs`

The starter registers `ExampleEcho` and a read-only TUI page. It needs no Node imports,
secrets, network, ambient files or build step. Rename `example.plugin` and the entry,
tool and app names together before creating a separately installed product.
The included composition patch activates its instance in the Profile root.

Minimal manifest without a default instance:

```json
{
  "schemaVersion": 1,
  "id": "example.plugin",
  "displayName": "Example plugin",
  "description": "Echo a value and show an example page.",
  "runtime": {"entry": "host.mjs", "sdkVersion": 3, "vm": "shared"}
}
```

Manifest fields are `schemaVersion`, `id`, `displayName`, `description`,
`dependencies: [{id}]`, `configuration`, `runtime`, `client`, and `composition`.
Unknown fields are rejected. `runtime` or `client` is required. Runtime/client
entries have `entry` and `sdkVersion`; runtime also has `vm: shared | dedicated`.
Use runtime plus `ctx.tui.app` for the native TUI: the optional `client` entry is a
separate client extension contract, not the terminal view factory.

`dependencies` names packages that must be present. Optional
`composition: {patch: "composition.json", structuralDependencies: [...]}` points
to a JSON/YAML operation array; the structural dependencies declare tree dependencies.
They are not npm imports. Required packages must have active entries in the same
composition root. Instance options belong in the entry's `config`, passed as the
second argument to `HostPlugin`; validate that JSON during activation. Manifest
`configuration` is not the currently active instance configuration.

Paths are relative package paths, not absolute paths,
`..`, links, or case-colliding names. Entries named by the manifest must exist.

## TypeScript and dependencies

Author TS against `@maka-agent/plugin-sdk/host`; use `import type` for HostPlugin,
HostContext, CallContext and data interfaces. In a Maka checkout build/typecheck
the existing workspace package. Outside it, use a matching local SDK checkout or
the complete shipped `references/sdk/*.ts` source set; do not assume the private
workspace package can be installed from npm. Preserve sibling module paths.
The full API is indexed in `references/api-map.md`.

Bundle each entry with a compatible bundler, for example after installing/pinning
esbuild in the plugin's development project:

```sh
npx esbuild src/host.ts --bundle --format=esm --platform=neutral --target=es2022 --outfile=dist/host.mjs
npx esbuild src/panel.ts --bundle --format=esm --platform=neutral --target=es2022 --outfile=dist/panel.mjs
```

Keep the runtime default export; bundling must remove imports, including dependency
imports. Await belongs inside exported functions. Dependencies using `node:fs`,
`process`, `fetch`, worker threads or ambient timers must be replaced with an
appropriate Host capability, not hidden behind a polyfill. Ship only the package
files referenced by the manifest and app declarations, not node_modules.
`dedicated` separates the package generation's VM from shared business VMs; it is
not a hostile-code sandbox and does not grant extra services.

## Install and activate

In the connected Host's plugin management page choose **Install package**, enter
the package directory on that Host, preview, then install. The immutable copy is
used afterward: editing your development directory does not hot-reload it.
Use the composition tree to add/enable/configure an instance if the package has no
default patch. Observe live activation status and any validation error.

The public protocol operations, for an authenticated client or integration test,
are `plugin.package.preview`, `plugin.package.install`, and
`plugin.composition.apply`. They are protocol names, not `maka plugin ...` CLI
commands. A minimal install input is `{sourcePath: absoluteHostDirectory}`.
A minimal explicit composition input is:

```json
{"operations":[{"type":"insert","rootId":"profile","entry":{"id":"example-plugin","packageId":"example.plugin"}}]}
```

Scope roots are `profile`, `desktop-ui`, and `session:<id>`. Entries carry `id`,
`packageId`, `config`, `disabled`, `inject`, `isolate`, `intercept`, `children`.
Profile and Session composition are distinct; use Session scope for an intentional
Session-specific instance, not as a way to acquire that Session's authority.
Use the public preview/editor for nontrivial composition rather than inventing
patch fields. Package installation may activate/restart entries automatically.
