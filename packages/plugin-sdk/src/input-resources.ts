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

import type { Awaitable, Cancellation, ReadDirectory } from './host.js';
import type { TerminalText } from './terminal-view.js';

/** Registration identity only; Host verifies current paired provider admission. */
export interface InputSelectionSource {
  readonly provider: string;
  readonly packageId: string;
  readonly entryId: string;
  readonly activation: string;
  readonly registration: string;
  readonly sessionId: string;
}
export interface InputResourceContext {
  readonly sessionId: string;
  readonly workspace: ReadDirectory;
  readonly signal: Cancellation;
}
export interface InputResourceQuery {
  readonly query: string;
  readonly cursor?: string | null;
  readonly limit: number;
  readonly locale: string;
}
export interface InputResourceResolve {
  readonly id: string;
  readonly locale: string;
}
export interface InputResourcePage {
  readonly items: readonly { id: string; title: string; description?: string }[];
  readonly nextCursor?: string | null;
}
export interface InputResourceValue {
  /** Must equal the selected catalog id; use versioned ids for mutable records. */
  readonly selector: string;
  readonly label: string;
  readonly quote?: {
    readonly text: string;
    readonly label?: string;
    readonly sourceTurnId?: string;
    readonly source?: {
      readonly sessionId: string;
      readonly sessionName: string;
      readonly capturedAt: number;
      readonly truncated: boolean;
    };
  } | null;
}
/** Opts this provider's resource selectors into queueable context semantics.
 * prepare must validate every selected resource and return ready or blocked.
 * Query/resolve observe only; they never dispatch or prepare user input.
 */
export interface InputResources {
  readonly title: TerminalText;
  query(request: InputResourceQuery, cx: InputResourceContext): Awaitable<InputResourcePage>;
  resolve(request: InputResourceResolve, cx: InputResourceContext): Awaitable<InputResourceValue>;
}
