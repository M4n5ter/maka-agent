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

export default async function activate(ctx) {
  let prepared = 0,
    pending = 0,
    cancelled = 0;
  for (let index = 0; index < 384; index++) await ctx.remote.method(`light-${index}`, () => index);
  await ctx.input.prepare(
    'example.context',
    (request) => {
      const selected = request.selections['example.context'];
      if (!selected) return { kind: 'unchanged' };
      if (selected.length !== 1 || selected[0] !== 'proof:0') {
        return { kind: 'blocked', message: 'Unknown context', receipt: { rejected: true } };
      }
      prepared++;
      return { kind: 'ready', content: request.content, receipt: { selected: selected[0] } };
    },
    {
      resources: {
        title: { fallback: 'Workspace proof' },
        async query(request, cx) {
          if (request.query === 'pending') {
            pending++;
            await cx.signal.wait();
            pending--;
            cancelled++;
            return { items: [] };
          }
          const start = Number(request.cursor ?? 0);
          return {
            items: [{ id: `proof:${start}`, title: 'Workspace proof' }],
            nextCursor: start === 0 ? '1' : null,
          };
        },
        async resolve(request, cx) {
          if (request.id !== 'proof:0') throw new Error('Unknown resource');
          const value = await cx.workspace.read({ path: 'proof.txt', limit: 64 });
          return {
            selector: 'proof:0',
            label: 'Workspace proof',
            quote: { text: new TextDecoder().decode(value.bytes) },
          };
        },
      },
    },
  );
  await ctx.remote.method('stats', () => ({ prepared, pending, cancelled }));
  await ctx.remote.method('registration-budget', async () => {
    const light = [];
    for (let index = 0; index < 320; index++)
      light.push(await ctx.remote.method(`dynamic-${index}`, () => index));
    const large = [];
    const route = 'x'.repeat(6000);
    let failure;
    const register = (name) =>
      ctx.tui.app(
        name,
        { entry: 'context-ui.mjs', backend: () => ({}) },
        {
          title: { fallback: 'Budget fixture' },
          context: 'application',
          commands: Array.from({ length: 9 }, (_, index) => ({
            name: `open-${index}`,
            title: { fallback: 'Open' },
            description: { fallback: 'Open existing route' },
            route,
          })),
        },
      );
    try {
      for (let index = 0; index < 1000; index++) large.push(await register(`large-${index}`));
    } catch (error) {
      failure = String(error.message);
    }
    const count = large.length;
    for (const registration of large) await registration.close();
    const reused = await register('reused-budget');
    await reused.close();
    for (const registration of light) await registration.close();
    return { light: light.length, large: count, failure, reused: true };
  });
}
