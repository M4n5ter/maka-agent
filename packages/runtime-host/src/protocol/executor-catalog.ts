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

import type { ExecutorCursor, ExecutorSearchResult } from '@maka-agent/plugin-sdk/host';
import { isExecutorId } from '@maka/core/executor-id';
import {
  requireCount,
  requireEncodedByteLimit,
  requireExactRecord,
  requireShapedRecord,
  requireUtf8String,
} from './codec.js';
import { invalidProtocolFrame } from './errors.js';
import { defineOperation } from './operation-spec.js';

export type ExecutorCatalogQuery = {
  scope?: 'profile' | 'desktop-ui' | `session:${string}`;
  query?: string;
  cursor?: ExecutorCursor | null;
};

export const EXECUTOR_CATALOG_OPERATION_SPECS = {
  'executor.catalog.query': defineOperation<
    ExecutorCatalogQuery,
    ExecutorSearchResult,
    | 'host_not_ready'
    | 'host_draining'
    | 'invalid_request'
    | 'operation_unavailable'
    | 'internal_failure'
    | 'persistence_failed'
  >({
    mode: 'query',
    availability: 'ready',
    errors: [
      'host_not_ready',
      'host_draining',
      'invalid_request',
      'operation_unavailable',
      'internal_failure',
      'persistence_failed',
    ],
    decodeInput(value) {
      const row = requireShapedRecord(value, 'executor query', [], ['scope', 'query', 'cursor']);
      let scope: ExecutorCatalogQuery['scope'];
      if (row.scope !== undefined) {
        if (row.scope === 'profile' || row.scope === 'desktop-ui') scope = row.scope;
        else if (
          typeof row.scope === 'string' &&
          row.scope.startsWith('session:') &&
          row.scope.length > 8 &&
          !/[\p{White_Space}\p{Cc}]/u.test(row.scope.slice(8))
        ) {
          requireUtf8String(row.scope.slice(8), 'executor session scope', 256);
          scope = row.scope as `session:${string}`;
        } else throw invalidProtocolFrame('Invalid executor scope');
      }
      const query =
        row.query === undefined ? undefined : requireUtf8String(row.query, 'executor search', 512);
      if (query && /\p{Cc}/u.test(query)) throw invalidProtocolFrame('Invalid executor search');
      return {
        ...(scope === undefined ? {} : { scope }),
        ...(query === undefined ? {} : { query }),
        ...(row.cursor === undefined ? {} : { cursor: decodeCursor(row.cursor) }),
      };
    },
    decodeOutput(value) {
      requireEncodedByteLimit(value, 'executor choices', 48 * 1024);
      const envelope = requireShapedRecord(value, 'executor search result', ['kind'], ['page']);
      if (envelope.kind === 'stale') {
        requireExactRecord(value, 'stale executor search', ['kind']);
        return { kind: 'stale' };
      }
      if (envelope.kind !== 'page') throw invalidProtocolFrame('Invalid executor search result');
      const page = requireExactRecord(envelope.page, 'executor choices', [
        'revision',
        'executors',
        'complete',
        'nextCursor',
      ]);
      if (
        !Array.isArray(page.executors) ||
        page.executors.length > 50 ||
        typeof page.complete !== 'boolean'
      )
        throw invalidProtocolFrame('Invalid executor choices');
      const names = new Set<string>();
      const executors = page.executors.map((value) => {
        const choice = requireExactRecord(value, 'executor choice', [
          'id',
          'displayName',
          'capabilities',
        ]);
        if (!isExecutorId(choice.id) || names.has(choice.id))
          throw invalidProtocolFrame('Invalid executor identifier');
        names.add(choice.id);
        const capabilities = requireExactRecord(choice.capabilities, 'executor capabilities', [
          'thinking',
          'toolActivity',
          'attachments',
          'historyCopy',
        ]);
        if (
          typeof capabilities.thinking !== 'boolean' ||
          typeof capabilities.toolActivity !== 'boolean' ||
          typeof capabilities.attachments !== 'boolean' ||
          typeof capabilities.historyCopy !== 'boolean'
        )
          throw invalidProtocolFrame('Invalid executor capabilities');
        return {
          id: choice.id,
          displayName: requireUtf8String(choice.displayName, 'executor display name', 48 * 1024),
          capabilities: {
            thinking: capabilities.thinking,
            toolActivity: capabilities.toolActivity,
            attachments: capabilities.attachments,
            historyCopy: capabilities.historyCopy,
          },
        };
      });
      const revision = requireCount(page.revision, 'executor revision');
      const nextCursor = decodeCursor(page.nextCursor);
      if (
        page.complete !== (nextCursor === null) ||
        (nextCursor && nextCursor.revision !== revision)
      )
        throw invalidProtocolFrame('Invalid executor continuation');
      return { kind: 'page', page: { revision, executors, complete: page.complete, nextCursor } };
    },
    assertOutputForInput(input, output) {
      if (output.kind === 'stale') return;
      const query = input.query ?? '';
      const scope = input.scope ?? 'profile';
      const original = input.cursor;
      if (
        original &&
        (original.query !== query ||
          original.scope !== scope ||
          original.revision !== output.page.revision)
      )
        throw invalidProtocolFrame('Executor page does not match its query');
      const next = output.page.nextCursor;
      if (
        next &&
        (output.page.executors.length === 0 ||
          next.query !== query ||
          next.scope !== scope ||
          next.offset !== (original?.offset ?? 0) + output.page.executors.length ||
          (original &&
            (next.generation !== original.generation || next.revision !== original.revision)))
      )
        throw invalidProtocolFrame('Executor continuation does not match its page');
    },
  }),
} as const;

function decodeCursor(value: unknown): ExecutorCursor | null {
  if (value === null) return null;
  const row = requireExactRecord(value, 'executor cursor', [
    'query',
    'generation',
    'revision',
    'scope',
    'offset',
  ]);
  const query = requireUtf8String(row.query, 'executor cursor query', 512);
  const generation = requireUtf8String(row.generation, 'executor cursor generation', 36);
  if (
    /\p{Cc}/u.test(query) ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(generation)
  )
    throw invalidProtocolFrame('Invalid executor cursor');
  const scope = requireUtf8String(row.scope, 'executor cursor scope', 264);
  if (
    scope !== 'profile' &&
    scope !== 'desktop-ui' &&
    !(
      scope.startsWith('session:') &&
      scope.length > 8 &&
      !/[\p{White_Space}\p{Cc}]/u.test(scope.slice(8))
    )
  )
    throw invalidProtocolFrame('Invalid executor cursor scope');
  return {
    query,
    generation,
    revision: requireCount(row.revision, 'executor cursor revision'),
    scope: scope as ExecutorCursor['scope'],
    offset: requireCount(row.offset, 'executor cursor offset'),
  };
}
