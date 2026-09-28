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

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { glob, readdir, readFile, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { nativeCliTargets } from './pack-cli.mjs';
import { rustTarget } from './build-cli.mjs';

const repository = fileURLToPath(new URL('../../', import.meta.url));
const documentPath = 'crates/cli/THIRD_PARTY_NOTICES.txt';
const manifestPath = 'crates/cli/notices.json';
const hash = (value) => createHash('sha256').update(value).digest('hex');
const licenseName = (name) =>
  (/^(?:licen[cs]e|copying|copyright|notice|credits)(?:$|[-_.])/i.test(name) ||
    /[-_]licen[cs]e(?:$|\.)/i.test(name)) &&
  !/\.(?:rs|py|h|cc|c|json|toml|sh|js)$/i.test(name);

async function inputs(root) {
  const names = [
    'Cargo.toml',
    'Cargo.lock',
    'package.json',
    'package-lock.json',
    'scripts/apply-dependency-patches.mjs',
    'scripts/rust/notices.mjs',
    'scripts/rust/bundle-providers.mjs',
    'scripts/rust/notices-sources.json',
    'crates/computer-use/THIRD_PARTY_NOTICES',
    'scripts/rust/v8-source.json',
    'scripts/rust/build-v8.mjs',
    'scripts/rust/v8-notices.mjs',
  ];
  for await (const entry of glob(
    [
      'crates/**/Cargo.toml',
      'crates/js-runtime/trusted/*.js',
      'crates/js-runtime/third-party/**/*',
      'patches/*.patch',
    ],
    { cwd: root, withFileTypes: true },
  )) {
    if (entry.isFile())
      names.push(relative(root, join(entry.parentPath, entry.name)).replaceAll('\\', '/'));
  }
  for (const entry of Object.values(
    JSON.parse(await readFile(join(root, 'scripts/rust/notices-sources.json'), 'utf8')),
  ).flat()) {
    if (entry.file) names.push(entry.file);
  }
  return Object.fromEntries(
    await Promise.all(
      [...new Set(names)]
        .sort()
        .map(async (name) => [name, hash(await readFile(join(root, name)))]),
    ),
  );
}

/** Release builds use only the reviewed, source-bound snapshot; no license downloads. */
export async function verifyNotices(root = repository) {
  const manifest = JSON.parse(await readFile(join(root, manifestPath), 'utf8'));
  const actual = await inputs(root);
  const document = await readFile(join(root, documentPath));
  if (
    JSON.stringify(manifest.inputs) !== JSON.stringify(actual) ||
    manifest.sha256 !== hash(document)
  ) {
    throw new Error(
      'Third-party notices are stale; run node scripts/rust/notices.mjs generate and review the changes',
    );
  }
  return join(root, documentPath);
}

export async function licenseFiles(directory, licenseDirectory = false) {
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.name === '.git' || entry.name === 'node_modules' || entry.name === 'target') continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory())
      found.push(
        ...(await licenseFiles(path, licenseDirectory || /^licen[cs]es?$/i.test(entry.name))),
      );
    else if (entry.isFile() && (licenseName(entry.name) || licenseDirectory)) found.push(path);
  }
  return found.sort();
}

function run(command, args, cwd) {
  return execFileSync(command, args, { cwd, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
}

export async function generateNotices(root = repository) {
  const groups = new Map();
  const downloaded = new Map();
  const supplements = JSON.parse(
    await readFile(join(root, 'scripts/rust/notices-sources.json'), 'utf8'),
  );
  function add(component, source, content) {
    content = content.replaceAll('\r\n', '\n').trim() + '\n';
    if (!content.trim() || content.includes('\0'))
      throw new Error('Invalid license text: ' + source);
    const key = hash(content);
    const group = groups.get(key) ?? { content, components: new Set(), sources: new Set() };
    group.components.add(component);
    group.sources.add(source);
    groups.set(key, group);
  }
  async function download(url) {
    if (!downloaded.has(url))
      downloaded.set(
        url,
        (async () => {
          const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
          if (!response.ok)
            throw new Error('License source failed: ' + url + ' (' + response.status + ')');
          return response.text();
        })(),
      );
    return downloaded.get(url);
  }
  const upstream = new Map();
  async function upstreamFiles(pkg, directory) {
    const vcs = JSON.parse(await readFile(join(directory, '.cargo_vcs_info.json'), 'utf8'));
    const repo = /^https:\/\/github\.com\/([^/]+\/[^/#]+?)(?:\.git)?(?:\/|$)/.exec(
      pkg.repository ?? '',
    )?.[1];
    const sha = vcs.git?.sha1;
    if (!repo || !/^[a-f0-9]{40}$/.test(sha))
      throw new Error('Missing pinned license source for ' + pkg.name);
    const key = repo + '/' + sha;
    if (!upstream.has(key))
      upstream.set(
        key,
        (async () => {
          const entries = JSON.parse(run('gh', ['api', `repos/${repo}/contents?ref=${sha}`], root));
          const licenses = entries.filter(
            (entry) => entry.type === 'file' && licenseName(entry.name),
          );
          for (const entry of entries.filter(
            (entry) => entry.type === 'dir' && /^licen[cs]es?$/i.test(entry.name),
          )) {
            licenses.push(
              ...JSON.parse(
                run('gh', ['api', `repos/${repo}/contents/${entry.path}?ref=${sha}`], root),
              ).filter((entry) => entry.type === 'file'),
            );
          }
          if (!licenses.length) throw new Error('No upstream license files for ' + key);
          return Promise.all(
            licenses.map(async (entry) => {
              const url = `https://raw.githubusercontent.com/${repo}/${sha}/${entry.path}`;
              return [url, await download(url)];
            }),
          );
        })(),
      );
    return upstream.get(key);
  }
  const packages = new Map();
  for (const platform of Object.values(nativeCliTargets)) {
    const metadata = JSON.parse(
      run(
        'cargo',
        [
          'metadata',
          '--locked',
          '--format-version',
          '1',
          '--filter-platform',
          rustTarget(platform.os, platform.cpu),
        ],
        root,
      ),
    );
    const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
    const catalog = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
    const pending = [metadata.packages.find((pkg) => pkg.name === 'maka-cli').id];
    const seen = new Set();
    while (pending.length) {
      const id = pending.pop();
      if (seen.has(id)) continue;
      seen.add(id);
      const pkg = catalog.get(id);
      if (pkg.source) packages.set(id, pkg);
      for (const dependency of nodes.get(id).deps) {
        if (dependency.dep_kinds.some((kind) => kind.kind !== 'dev')) pending.push(dependency.pkg);
      }
    }
  }
  const failures = [];
  for (const pkg of [...packages.values()].sort((a, b) =>
    (a.name + a.version).localeCompare(b.name + b.version),
  )) {
    try {
      const component =
        `${pkg.name}@${pkg.version} (${pkg.license ?? 'MIT; see upstream notice'})` +
        (pkg.authors.length ? `; authors declared by package: ${pkg.authors.join(', ')}` : '');
      const directory = dirname(pkg.manifest_path);
      const source = pkg.source.startsWith('registry+')
        ? `https://crates.io/api/v1/crates/${pkg.name}/${pkg.version}/download`
        : pkg.source;
      let local = await licenseFiles(directory);
      if (pkg.license_file && !local.includes(pkg.license_file)) local.push(pkg.license_file);
      let references = false;
      local = (
        await Promise.all(
          local.map(async (file) => {
            const text = await readFile(file, 'utf8');
            if (text.trim().length < 80) {
              references ||= text.includes('../');
              return null;
            }
            return file;
          }),
        )
      ).filter(Boolean);
      if (pkg.source.includes('github.com/M4n5ter/cua')) {
        add(
          component,
          source,
          await readFile(join(root, 'crates/computer-use/THIRD_PARTY_NOTICES'), 'utf8'),
        );
      } else if (supplements[pkg.name + '@' + pkg.version]) {
        for (const entry of supplements[pkg.name + '@' + pkg.version]) {
          if (typeof entry === 'string') add(component, entry, await download(entry));
          else add(component, entry.source, await readFile(join(root, entry.file), 'utf8'));
        }
      } else if (!local.length || references) {
        for (const [url, content] of await upstreamFiles(pkg, directory))
          add(component, url, content);
      }
      for (const file of local)
        add(
          component,
          source + ' :: ' + relative(directory, file).replaceAll('\\', '/'),
          await readFile(file, 'utf8'),
        );
    } catch (error) {
      failures.push(pkg.name + '@' + pkg.version + ': ' + error.message);
    }
  }
  if (failures.length) throw new Error(failures.join('\n'));
  const stage = await mkdtemp(join(tmpdir(), 'maka-notices-'));
  try {
    const meta = join(stage, 'inputs.json');
    run(process.execPath, ['scripts/rust/bundle-providers.mjs', stage, meta], root);
    const bundled = new Set();
    for (const input of Object.keys(JSON.parse(await readFile(meta, 'utf8')).inputs)) {
      let path = dirname(resolve(root, input));
      if (!path.includes('node_modules')) continue;
      while (true) {
        const manifest = await readFile(join(path, 'package.json'), 'utf8')
          .then(JSON.parse)
          .catch(() => null);
        if (manifest?.name && manifest.version) break;
        if (dirname(path) === path) throw new Error('Cannot identify bundled package: ' + input);
        path = dirname(path);
      }
      bundled.add(path);
    }
    for (const directory of [...bundled].sort()) {
      const pkg = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
      const local = await licenseFiles(directory);
      if (!local.length) throw new Error('Missing bundled JavaScript license: ' + pkg.name);
      for (const url of supplements[pkg.name + '@' + pkg.version] ?? [])
        add(`${pkg.name}@${pkg.version} (${pkg.license})`, url, await download(url));
      for (const file of local)
        add(
          `${pkg.name}@${pkg.version} (${pkg.license})`,
          `https://registry.npmjs.org/${pkg.name}/-/${pkg.name.split('/').at(-1)}-${pkg.version}.tgz :: ${relative(directory, file).replaceAll('\\', '/')}`,
          await readFile(file, 'utf8'),
        );
    }
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
  const sections = [...groups.values()]
    .map((group) =>
      [
        [...group.components].sort().join('\n'),
        [...group.sources].sort().join('\n'),
        group.content,
      ].join('\n\n'),
    )
    .sort();
  const content =
    'Third-party notices for the Maka native distribution\n\n' +
    'Dependency sources and alternative license texts are preserved below. Source URLs identify the exact distributed versions.\n\n' +
    sections.join('\n' + '='.repeat(80) + '\n\n');
  await writeFile(join(root, documentPath), content);
  await writeFile(
    join(root, manifestPath),
    JSON.stringify({ inputs: await inputs(root), sha256: hash(content) }, null, 2) + '\n',
  );
  return { components: packages.size, texts: groups.size, bytes: Buffer.byteLength(content) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.length !== 3 || !['check', 'generate'].includes(process.argv[2]))
    throw new Error('Usage: notices.mjs <check|generate>');
  console.log(process.argv[2] === 'check' ? await verifyNotices() : await generateNotices());
}
