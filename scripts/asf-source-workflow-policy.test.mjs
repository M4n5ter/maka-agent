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

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { parse } from 'yaml';

const workflow = parse(
  readFileSync(new URL('../.github/workflows/asf-source-candidate.yml', import.meta.url), 'utf8'),
);
const steps = workflow.jobs.candidate.steps;

test('source candidates bind their archive to the dispatched commit', () => {
  assert.equal(workflow.permissions.contents, 'read');
  const checkout = steps.find((step) => step.uses?.startsWith('actions/checkout@'));
  assert.equal(checkout.with.ref, '${{ github.sha }}');
  assert.equal(checkout.with['persist-credentials'], false);
  assert.equal(
    steps.find((step) => step.name === 'Create source candidate').run,
    'just source "$RELEASE_VERSION" "$GITHUB_SHA"',
  );
  assert.match(
    steps.find((step) => step.name === 'Extract exact candidate').run,
    /tar -xzf "\$CANDIDATE_PATH" --strip-components=1 -C candidate-source/,
  );
  assert.equal(
    steps.find((step) => step.uses?.startsWith('actions/upload-artifact@')).with.path,
    'release/asf/*',
  );
});

test('validation reads extracted source and audits it before installation', () => {
  const names = steps.map((step) => step.name);
  assert.ok(
    names.indexOf('Audit extracted source') < names.indexOf('Install extracted dependencies'),
  );
  for (const name of [
    'Audit extracted source',
    'Install extracted dependencies',
    'Verify extracted source',
    'Verify website',
  ]) {
    assert.equal(steps.find((step) => step.name === name)['working-directory'], 'candidate-source');
  }
  assert.match(steps.find((step) => step.name === 'Verify extracted source').run, /just check/);
});
