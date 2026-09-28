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

# Security

Report vulnerabilities privately to **security@maka.app**, including the Maka version, affected crate, operating system and a minimal reproduction. Do not disclose vulnerabilities in public issues.

## Boundaries

The Runtime Host owns operation admission, execution scopes, canonical state and resource settlement. Clients and model output cannot grant themselves capabilities. Plugin invocations use the capabilities granted to their owner and current execution scope.

Native tools enforce filesystem, process and network restrictions through the relevant operating-system mechanisms. Approval grants a specific capability; it does not validate untrusted text or make a stale observation executable.

Code Mode executes JavaScript in an embedded V8 isolate with explicit host bindings. Installed Host plugins are trusted extensions. Computer Use has a separate session and observation lifecycle, and requires the corresponding platform permissions.

Configuration and event data remain under the selected state root. Secrets are handled through credential capabilities; plugin state, tool output and application UI must not be treated as secret stores.

## Distribution

Native npm packages are checked against their target, version, archive integrity and source identity before publication. The launcher selects an exact-version platform package. Source archives have SHA-512 sidecars; ASF release signatures and source-review instructions are documented in [.github/ASF_SOURCE_RELEASE.md](.github/ASF_SOURCE_RELEASE.md).
