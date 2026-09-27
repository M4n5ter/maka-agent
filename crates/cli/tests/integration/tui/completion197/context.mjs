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
  const provider = 'example.completion197';
  const stats = { queries: 0, resolved: 0, prepared: 0, pending: 0, cancelled: 0, sources: [] };
  await ctx.input.prepare(
    provider,
    (request) => {
      const selected = request.selections[provider];
      if (!selected) return { kind: 'unchanged' };
      if (selected.length !== 1 || selected[0] !== 'proof:197') {
        return { kind: 'blocked', message: 'Unknown resource', receipt: { invalid: true } };
      }
      stats.prepared++;
      stats.sources = request.selectionSources;
      return {
        kind: 'ready',
        content: {
          ...request.content,
          text: `${request.content.text}\nPREPARED_PLUGIN_SOURCE_197`,
        },
        receipt: { selected: selected[0], sources: request.selectionSources },
      };
    },
    {
      resources: {
        title: { fallback: 'PTY context source' },
        async query(request, cx) {
          stats.queries++;
          if (request.query === 'slow') {
            stats.pending++;
            try {
              await cx.signal.wait();
            } finally {
              stats.pending--;
              stats.cancelled++;
            }
            return { items: [] };
          }
          return {
            items: [
              {
                id: 'proof:197',
                title: 'Captured plugin evidence',
                description: 'Read-only workspace excerpt',
              },
            ],
          };
        },
        async resolve(request, cx) {
          if (request.id !== 'proof:197') throw new Error('Unknown resource');
          const bytes = await cx.workspace.read({ path: 'plugin-proof.txt', limit: 4096 });
          stats.resolved++;
          return {
            selector: 'proof:197',
            label: 'Captured plugin evidence',
            quote: { text: new TextDecoder().decode(bytes.bytes), label: 'Plugin proof' },
          };
        },
      },
    },
  );
  await ctx.remote.method('stats', () => ({ ...stats }));
}
