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

import { execFileSync, spawn } from 'node:child_process';
import { constants } from 'node:fs';
import { access, mkdir, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { rustTarget } from './build-cli.mjs';
import { cachedV8, v8CacheKey } from './v8-cache.mjs';
import { v8Notices } from './v8-notices.mjs';

/** Build the locked V8 without its optional LGPL glibc implementation. */
export async function buildV8({ root, directory, cache, target, env, metadata }) {
  const configuration = await v8Configuration(root, target, metadata);
  return cachedV8({
    directory: cache,
    key: configuration.key,
    target,
    build: () => compileV8({ ...configuration, directory, target, env }),
  });
}

async function v8Configuration(root, target, metadata) {
  const source = JSON.parse(await readFile(join(root, 'scripts/rust/v8-source.json'), 'utf8'));
  const pkg = metadata.packages.find((pkg) => pkg.name === 'v8');
  if (pkg?.version !== source.version) throw new Error('V8 source pin differs from Cargo.lock');
  const features = metadata.resolve.nodes
    .find((node) => node.id === pkg.id)
    .features.filter((name) => name !== 'default')
    .sort();
  return { source, features, key: await v8CacheKey({ source, target, features }) };
}

async function compileV8({ source, features, directory, target, env }) {
  const checkout = join(directory, 'source');
  const output = join(directory, 'target');
  await mkdir(checkout, { recursive: true });
  const run = async (command, args, variables = env) =>
    new Promise((resolve, reject) => {
      const child = spawn(command, args, { cwd: checkout, env: variables, stdio: 'inherit' });
      child.once('error', reject);
      child.once('exit', (code, signal) =>
        code === 0
          ? resolve()
          : reject(new Error(`V8 source build failed: ${command} (${signal ?? code})`)),
      );
    });
  // These options also reach recursive submodule checkouts on Windows.
  const git = (...args) =>
    run('git', ['-c', 'core.symlinks=true', '-c', 'core.longpaths=true', ...args]);
  await git('init', '--quiet');
  await git('fetch', '--depth=1', source.repository, source.revision);
  await git('checkout', '--quiet', '--detach', 'FETCH_HEAD');
  await git('submodule', 'update', '--init', '--recursive', '--depth=1');
  const buildDirectory = join(output, target, 'release');
  const python = execFileSync(
    env.PYTHON || 'python3',
    ['-c', 'import sys; print(sys.executable)'],
    {
      env,
      encoding: 'utf8',
    },
  ).trim();
  // Select the tools pinned by this checkout, even when PATH contains GN/Ninja.
  const tools = join(buildDirectory, 'ninja_gn_binaries');
  await run(python, ['tools/ninja_gn_binaries.py', '--dir', tools]);
  const clang = join(buildDirectory, 'clang');
  await run(python, ['tools/clang/scripts/update.py', '--output-dir', clang]);
  const windows = process.platform === 'win32';
  // Chromium ships its Windows librarian through lld-link /lib, not llvm-ar.
  const archiver = join(clang, 'bin', windows ? 'lld-link.exe' : 'llvm-ar');
  await access(archiver, constants.X_OK);
  let gnArgs = source.gnArgs;
  const bindingEnvironment = {};
  if (target === 'x86_64-unknown-linux-gnu') {
    // Chromium's sysroot headers preserve the glibc baseline used by Zig below.
    await run(python, ['build/linux/sysroot_scripts/install-sysroot.py', '--arch=amd64']);
    gnArgs += ' use_sysroot=true';
    bindingEnvironment.BINDGEN_EXTRA_CLANG_ARGS =
      '--sysroot=' + JSON.stringify(join(checkout, 'build/linux/debian_bullseye_amd64-sysroot'));
  }
  const suffix = windows ? '.exe' : '';
  const buildEnvironment = {
    ...env,
    ...bindingEnvironment,
    // This cache contains ordinary release V8, never an ambient ASAN variant.
    CARGO_ENCODED_RUSTFLAGS: '',
    PYTHON: python,
    GN: join(tools, 'gn', 'gn' + suffix),
    NINJA: join(tools, 'ninja', 'ninja' + suffix),
    DEPOT_TOOLS_WIN_TOOLCHAIN: '0',
    V8_FROM_SOURCE: '1',
    GN_ARGS: gnArgs,
  };
  await run(
    'cargo',
    [
      '+stable',
      'build',
      '--release',
      '--locked',
      '--lib',
      '--target',
      target,
      '--target-dir',
      output,
      '--no-default-features',
      '--features',
      features.join(','),
    ],
    buildEnvironment,
  );
  const generated = join(buildDirectory, 'gn_out');
  const archive = join(
    generated,
    'obj',
    target.includes('windows') ? 'rusty_v8.lib' : 'librusty_v8.a',
  );
  const members = execFileSync(archiver, [...(windows ? ['/lib', '/list'] : ['t']), archive], {
    encoding: 'utf8',
    maxBuffer: 4 * 1024 * 1024,
  });
  if (/\b(?:branred|s_sin)\.(?:o|obj)\b/.test(members))
    throw new Error('V8 archive still contains vendored glibc objects');
  return {
    environment: {
      RUSTY_V8_ARCHIVE: archive,
      RUSTY_V8_SRC_BINDING_PATH: join(generated, 'src_binding.rs'),
    },
    notices: await v8Notices(checkout, generated, source, buildEnvironment),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length !== 3 || process.argv[2] !== '--cache-key')
    throw new Error('Usage: build-v8.mjs --cache-key');
  const root = resolve(import.meta.dirname, '../..');
  const target = rustTarget();
  const metadata = JSON.parse(
    execFileSync(
      'cargo',
      ['metadata', '--locked', '--format-version', '1', '--filter-platform', target],
      { cwd: root, maxBuffer: 16 * 1024 * 1024, timeout: 180_000 },
    ),
  );
  console.log((await v8Configuration(root, target, metadata)).key);
}
