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
import { test } from 'node:test';
import { fixture } from './terminal-transcript-harness.mjs';

test('remote observations and canonical controls capture the same original caller without mixing their interfaces', async () => {
  const f = await fixture({}, async (ctx) => {
    await ctx.remote.method('controls', async (_input, cx) => {
      assert.ok(Object.isFrozen(cx.views));
      assert.ok(Object.isFrozen(cx.controls));
      assert.equal(cx.views.updatePreferences, undefined);
      assert.equal(cx.views.createExecutorSession, undefined);
      assert.equal(cx.views.configureExecutorSession, undefined);
      const forged = { ...cx, remoteAuthority: 'forged-authority', sessionId: 'forged-session' };
      await forged.views.preferences();
      await forged.views.executorSession('observed-session');
      await forged.views.executorCreation({ sessionId: 'original-operation' });
      await forged.controls.updatePreferences({
        expectedRevision: 3,
        mutation: { kind: 'workspace_instructions', enabled: false },
      });
      await forged.controls.createExecutorSession({ sessionId: 'original-operation' });
      await forged.controls.configureExecutorSession({
        expectedRevision: 5,
        executorId: 'example',
      });
      return 'done';
    });
  });
  const definition = f.registrations[0];
  const reply = await f.runtime.invoke(definition.callback, null, {
    remoteAuthority: 'captured-authority',
    sessionId: 'bound-session',
  });
  assert.equal(reply.kind, 'value', reply.message);
  assert.equal(reply.value, 'done');
  assert.equal(f.operations.length, 6);
  assert.ok(f.operations.every(({ input }) => input.authority === 'captured-authority'));
  assert.deepEqual(
    f.operations.map(({ method }) => method),
    [
      'remote.preferences',
      'remote.executorSession',
      'remote.executorCreation',
      'remote.updatePreferences',
      'remote.createExecutorSession',
      'remote.configureExecutorSession',
    ],
  );
});
