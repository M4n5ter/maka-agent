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

test('input discovery shares its provider registration and carries only readonly lookup context', async () => {
  let prepares = 0;
  const f = await fixture({}, async (ctx) => {
    await ctx.input.prepare(
      'example.context',
      (request) => {
        prepares++;
        assert.equal(request.selections['example.context'][0], 'record:7');
        assert.equal(request.selectionSources[0].registration, 'original-registration');
        return { kind: 'ready', content: request.content, receipt: { selected: true } };
      },
      {
        resources: {
          title: { fallback: 'Context records' },
          query(request, cx) {
            assert.deepEqual(Object.keys(cx).sort(), [
              'cwd',
              'sessionId',
              'signal',
              'tools',
              'workspace',
            ]);
            assert.equal(cx.sessionId, 'session');
            assert.equal(cx.cwd, '/workspace');
            assert.deepEqual(Array.from(cx.tools), ['Skill']);
            assert.ok(Object.isFrozen(cx.tools));
            assert.equal(cx.workspace.write, undefined);
            assert.equal(request.query, 'rec');
            assert.equal(prepares, 0, 'typing must not enter input preparation');
            return { items: [{ id: 'record:7', title: 'Record seven' }], nextCursor: 'page-2' };
          },
          resolve(request, cx) {
            assert.equal(cx.sessionId, 'session');
            assert.equal(cx.cwd, '/workspace');
            assert.deepEqual(Array.from(cx.tools), ['Skill']);
            assert.ok(Object.isFrozen(cx.tools));
            assert.equal(request.id, 'record:7');
            assert.equal(prepares, 0, 'choosing context does not submit it');
            return {
              selector: 'record:7',
              label: 'Record seven',
              quote: { text: 'captured bytes' },
            };
          },
        },
      },
    );
  });
  assert.equal(f.registrations.length, 1);
  const definition = f.registrations[0];
  assert.equal(definition.kind, 'input_resources');
  assert.equal(definition.descriptor.title.fallback, 'Context records');
  const invoke = (operation, input) =>
    f.runtime.invoke(definition.callback, input, {
      inputResource: operation,
      readView: 'readonly-view',
      cwd: '/workspace',
      tools: ['Skill'],
      sessionId: 'session',
    });
  const page = await invoke('query', { query: 'rec', limit: 32, locale: 'en' });
  assert.equal(page.nextCursor, 'page-2');
  const chosen = await invoke('resolve', { id: 'record:7', locale: 'en' });
  assert.equal(chosen.selector, 'record:7');
  assert.equal(chosen.quote.text, 'captured bytes');
  await f.runtime.invoke(
    definition.callback,
    {
      content: { text: 'message' },
      selections: { 'example.context': ['record:7'] },
      selectionSources: [{ registration: 'original-registration' }],
    },
    { readView: 'readonly-view' },
  );
  assert.equal(prepares, 1);
  assert.equal(f.operations.length, 0, 'lookup has not acquired or invoked execution authority');
  await f.runtime.dispose();
});

test('each pending lookup receives the existing exact invocation cancellation signal', async () => {
  let entered;
  const started = new Promise((resolve) => {
    entered = resolve;
  });
  const f = await fixture({}, async (ctx) => {
    await ctx.input.prepare('example.context', () => ({ kind: 'unchanged' }), {
      resources: {
        title: { fallback: 'Context' },
        async query(_request, cx) {
          entered();
          await cx.signal.wait();
          assert.equal(cx.signal.aborted, true);
          return { items: [] };
        },
        resolve: () => ({ selector: 'item', label: 'Item' }),
      },
    });
  });
  const pending = f.runtime.invoke(
    f.registrations[0].callback,
    { query: '', limit: 32, locale: 'en' },
    {
      inputResource: 'query',
      readView: 'view',
      sessionId: 'session',
    },
    'lookup-1',
  );
  await started;
  f.runtime.cancel('lookup-1');
  assert.equal((await pending).items.length, 0);
  await f.runtime.dispose();
});

test('incomplete resource callbacks are rejected before publication', async () => {
  await assert.rejects(
    fixture({}, async (ctx) => {
      await ctx.input.prepare('bad', () => ({ kind: 'unchanged' }), {
        resources: {
          title: { fallback: 'Bad' },
          query: () => ({ items: [] }),
        },
      });
    }),
    /require query and resolve/,
  );
});
