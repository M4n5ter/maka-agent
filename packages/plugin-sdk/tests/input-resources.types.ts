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

import type { HostContext, InputSelectionSource, TerminalCommand } from '../src/host.js';
declare const ctx: HostContext;
const command: TerminalCommand = {
  name: 'review',
  title: { fallback: 'Review' },
  description: { fallback: 'Open review form' },
  route: { page: 'review' },
};
void command;
void ctx.input.prepare(
  'example.context',
  (request) => {
    const source: InputSelectionSource | undefined = request.selectionSources[0];
    void source;
    return { kind: 'unchanged' };
  },
  {
    resources: {
      title: { fallback: 'Context' },
      async query(request, cx) {
        await cx.workspace.list({ limit: request.limit });
        // @ts-expect-error Lookup has no writing capability.
        cx.workspace.write({ path: 'escape', bytes: [] });
        // @ts-expect-error Lookup is not a general Remote caller.
        cx.views.session();
        return { items: [{ id: 'one', title: 'One' }] };
      },
      resolve: (request) => ({
        selector: request.id,
        label: 'One',
        quote: { text: 'Immutable capture' },
      }),
    },
  },
);
