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
import test from 'node:test';
import { EXECUTOR_CATALOG_OPERATION_SPECS } from '../protocol/executor-catalog.js';

const spec = EXECUTOR_CATALOG_OPERATION_SPECS['executor.catalog.query'];
const cursor = {
  query: 'same',
  generation: '12345678-1234-1234-1234-123456789012',
  scope: 'session:a' as const,
  revision: 9,
  offset: 50,
};
const choice = {
  id: 'example.agent',
  displayName: 'Same agent',
  capabilities: { thinking: false, toolActivity: false, attachments: false, historyCopy: false },
};

test('executor pages bind their exact query, scope and progressing cursor', () => {
  const assertOutputForInput = spec.assertOutputForInput;
  assert.ok(assertOutputForInput);
  const input = spec.decodeInput({ query: 'same', scope: 'session:a', cursor });
  const output = spec.decodeOutput({
    kind: 'page',
    page: {
      revision: 9,
      executors: [choice],
      complete: false,
      nextCursor: { ...cursor, offset: 51 },
    },
  });
  assertOutputForInput(input, output);
  for (const patch of [
    { query: 'different' },
    { scope: 'session:b' },
    { offset: 50 },
    { generation: '87654321-1234-1234-1234-123456789012' },
  ]) {
    const wrong = spec.decodeOutput({
      kind: 'page',
      page: {
        revision: 9,
        executors: [choice],
        complete: false,
        nextCursor: { ...cursor, offset: 51, ...patch },
      },
    });
    assert.throws(() => assertOutputForInput(input, wrong));
  }
  assert.deepEqual(spec.decodeOutput({ kind: 'stale' }), { kind: 'stale' });
  assert.throws(() => spec.decodeOutput({ kind: 'stale', page: {} }));
  assert.throws(() =>
    spec.decodeOutput({
      kind: 'page',
      page: { revision: 9, executors: [], complete: true, nextCursor: cursor },
    }),
  );
});
