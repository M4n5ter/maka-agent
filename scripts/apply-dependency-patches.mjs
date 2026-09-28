#!/usr/bin/env node
/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

// Apply the exact-version provider SDK patches used by native builds.
// Release builds install the complete locked dependency tree. A types-only
// workspace install may omit the root development tooling.
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const patchesDirectory = join(repoRoot, 'patches');

if (
  !existsSync(patchesDirectory) ||
  !readdirSync(patchesDirectory).some((entry) => entry.endsWith('.patch'))
) {
  process.exit(0);
}

let patchPackageEntry;
try {
  patchPackageEntry = createRequire(import.meta.url).resolve('patch-package/index.js');
} catch {
  console.warn(
    'patch-package is not installed; skipping patches/. Run a plain `npm ci` from the repo root before building or packaging.',
  );
  process.exit(0);
}

const result = spawnSync(process.execPath, [patchPackageEntry, '--error-on-fail'], {
  cwd: repoRoot,
  stdio: 'inherit',
});
process.exit(result.status ?? 1);
