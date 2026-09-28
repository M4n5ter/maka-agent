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

# Maka npm launcher

[简体中文](README.zh-CN.md)

This package starts the native Maka CLI for the current OS and CPU. Exact-version optional dependencies carry the executables. Arguments, standard input/output and exit status pass through to the native process.

The supported preview set is macOS arm64, Linux x64 with glibc, and Windows x64. WSL uses the Linux executable and can obtain the matching Windows helper through the native distribution layer.

## Build and publish

Create a committed source candidate with `just source <version>`. Each platform runs:

```sh
just package <source.tar.gz> <target> <build-id> native-preview
```

Collect the platform archives, receipts and source archive with its SHA-512 sidecar in one directory. `just release-prepare native-preview` packages the launcher from that verified source and validates the complete set. `just publish native-preview` publishes the native packages first and the launcher last, using the `rust-preview` npm tag.

No install script downloads or executes an unverified helper. Runtime behavior lives in `crates/cli`.
