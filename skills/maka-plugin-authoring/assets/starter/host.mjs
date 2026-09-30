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
  await ctx.tools.register(
    {
      name: 'ExampleEcho',
      description: 'Return the supplied text unchanged.',
      inputSchema: {
        type: 'object',
        properties: { text: { type: 'string', maxLength: 4096 } },
        required: ['text'],
        additionalProperties: false,
      },
      semantics: 'parallel',
    },
    ({ text }, call) => {
      call.signal.throwIfAborted();
      if (typeof text !== 'string' || text.length > 4096)
        throw new Error('text must be at most 4096 characters');
      return { text };
    },
  );
  await ctx.tui.app(
    'overview',
    {
      entry: 'panel.mjs',
      async backend(request) {
        if (request.kind === 'read') return { message: 'ExampleEcho is ready.' };
        return { kind: 'rejected', message: 'This example is read-only.' };
      },
    },
    {
      title: { fallback: 'Plugin example', translations: { 'zh-CN': '插件示例' } },
      context: 'application',
    },
  );
}
