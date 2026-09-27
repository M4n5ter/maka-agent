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

export async function verifyRequestHeaders(
  connection,
  peer,
  provider,
  connectionId,
  model,
  overlay,
) {
  const call = (op, input) => connection.request(op, input, 3000);
  const query = (id = connectionId) =>
    call('connection.request-headers.query', { connectionId: id });
  const replace = async (headers, writer = connection) => {
    const { basis: expected } = await query();
    return writer.request('connection.request-headers.replace', { expected, headers }, 3000);
  };
  const expectHeaders = async (result, kind, names) => {
    const { basis } = result;
    assert.deepEqual(result, { kind, names, basis });
    assert.equal(basis.connection.connectionId, connectionId);
    assert(Number.isSafeInteger(basis.connection.revision) && basis.connection.revision > 0);
    if (basis.credential !== null) {
      assert.deepEqual(basis.credential.locator, {
        scope: 'connection',
        connectionId,
        kind: 'request_headers',
      });
      assert.equal(typeof basis.credential.credentialId, 'string');
      assert(Number.isSafeInteger(basis.credential.revision) && basis.credential.revision > 0);
    }
    assert.deepEqual(await query(), { kind: 'found', names, basis });
    return basis;
  };
  const catalog = () => call('connection.catalog.query', { kind: 'start' });
  const names = ['X-Maka-Retained'];
  const missing = '12345678-1234-4234-8234-123456789abc';
  assert.deepEqual(await query(missing), { kind: 'connection_not_found' });
  assert.deepEqual(
    await call('connection.request-headers.replace', {
      expected: { connection: { connectionId: missing, revision: 1 }, credential: null },
      headers: [],
    }),
    { kind: 'connection_not_found' },
  );
  await expectHeaders(await query(), 'found', []);
  const notices = [];
  const unsubscribe = connection.subscribeConfigurationChanges((value) => notices.push(value));
  const beforeCount = provider.count;
  try {
    await expectHeaders(
      await replace([
        { name: '\uFEFF X-Maka-Retained ', value: 'private-header' },
        { name: 'X-Remove', value: 'remove' },
      ]),
      'committed',
      [...names, 'X-Remove'],
    );
    await expectHeaders(await query(), 'found', [...names, 'X-Remove']);
    await assert.rejects(replace([{ name: 'X-New' }]), { code: 'invalid_request' });
    await expectHeaders(await replace([{ name: names[0] }]), 'committed', names);
    provider.auth(false);
    provider.expect('openai-compatible', model, overlay);
    provider.headers('private-header');
    const run = () => call('connection.test.run', { connectionId, modelId: model });
    assert.equal((await run()).test.kind, 'verified');
    const before = await catalog(),
      notificationCount = notices.length;
    const unchangedBasis = (await query()).basis;
    assert.deepEqual(
      await expectHeaders(await replace([{ name: names[0] }]), 'unchanged', names),
      unchangedBasis,
    );
    assert.deepEqual(
      await catalog(),
      before,
      'unchanged headers retain verification and catalog revision',
    );
    assert.equal(notices.length, notificationCount, 'unchanged headers publish no invalidation');

    const arrived = provider.hold(),
      pending = run();
    const release = await arrived;
    try {
      await expectHeaders(
        await replace([{ name: names[0], value: 'changed' }], peer),
        'committed',
        names,
      );
    } finally {
      release();
    }
    assert.deepEqual(await pending, { kind: 'superseded', changed: ['credential'] });
    const changed = await catalog();
    assert.equal(
      changed.items.find((item) => item.kind === 'connection' && item.connectionId === connectionId)
        .lastTest,
      undefined,
    );
    await expectHeaders(await replace([]), 'committed', []);
    await expectHeaders(await replace([]), 'unchanged', []);
    await expectHeaders(await query(), 'found', []);
    await expectHeaders(
      await replace([{ name: names[0], value: 'reopened-header' }]),
      'committed',
      names,
    );
    provider.headers('reopened-header');
    assert.equal((await run()).test.kind, 'verified');
    assert.equal(provider.count, beforeCount + 3);
    await expectHeaders(await query(), 'found', names);
  } finally {
    unsubscribe();
  }
}
