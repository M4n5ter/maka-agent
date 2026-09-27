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

import { useEffect, useState } from 'react';
import { Button } from '@maka/ui/plugin';
import type { ClientContext } from '@maka-agent/plugin-sdk/client';
import type {
  ExecutionTarget,
  ModelChoices,
  ModelCursor,
  ModelSearch,
  ModelSearchResult,
} from '@maka-agent/plugin-sdk/host';

export type ModelTarget = Extract<ExecutionTarget, { kind: 'model' }>;

export function ModelSelection({
  context,
  locale,
  label,
  onSelect,
  initialTarget,
}: {
  context: ClientContext;
  locale: string;
  label: string;
  onSelect: (target: ModelTarget) => Promise<void>;
  initialTarget?: ModelTarget;
}) {
  const zh = locale !== 'en';
  const [query, setQuery] = useState('');
  const [history, setHistory] = useState<(ModelCursor | null)[]>([null]);
  const cursor = history[history.length - 1];
  const [revision, setRevision] = useState(0);
  const t = (en: string, cn: string, tw: string) => (locale === 'zh-TW' ? tw : zh ? cn : en);
  const [choices, setChoices] = useState<ModelChoices>();
  const [selected, setSelected] = useState(() =>
    initialTarget ? JSON.stringify(initialTarget.model) : '',
  );
  const [thinking, setThinking] = useState<{ model: string; level: string } | undefined>(() =>
    initialTarget
      ? {
          model: JSON.stringify(initialTarget.model),
          level: initialTarget.thinkingLevel ?? '',
        }
      : undefined,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    let active = true;
    setChoices(undefined);
    setError(undefined);
    const timer = setTimeout(() => {
      void context.remote
        .method<ModelSearch, ModelSearchResult>('models')({ query, cursor })
        .then((page) => {
          if (!active) return;
          if (page.kind === 'page') setChoices(page.page);
          else
            setError(
              locale === 'en'
                ? 'Choices changed. Refresh to search again.'
                : locale === 'zh-TW'
                  ? '選項已變更，請重新整理後搜尋。'
                  : '选项已变化，请刷新后搜索。',
            );
        })
        .catch((error: unknown) => {
          if (active) setError(error instanceof Error ? error.message : String(error));
        });
    }, 150);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [context, query, cursor, revision, locale]);
  const key = (choice: ModelChoices['models'][number]) => JSON.stringify(choice.model);
  const model =
    choices?.models.find((choice) => key(choice) === selected) ??
    choices?.models.find((choice) => choice.isDefault) ??
    choices?.models[0];
  const level =
    model && thinking?.model === key(model)
      ? model.thinkingLevels.find((level) => level === thinking.level)
      : model?.defaultThinkingLevel;
  return (
    <div className="workhub-model-selection">
      <label>
        {zh ? '搜索模型' : 'Search models'}
        <input
          value={query}
          disabled={busy}
          onChange={(event) => {
            setQuery(event.target.value);
            setHistory([null]);
          }}
        />
      </label>
      <label>
        {zh ? '模型' : 'Model'}
        <select
          value={model ? key(model) : ''}
          disabled={busy || !model}
          onChange={(event) => {
            setSelected(event.target.value);
            setThinking(undefined);
          }}
        >
          {!model ? <option value="">{zh ? '没有可用模型' : 'No models available'}</option> : null}
          {choices?.models.map((choice) => (
            <option key={key(choice)} value={key(choice)}>
              {choice.connectionName} · {choice.displayName}
            </option>
          ))}
        </select>
      </label>
      {model?.thinkingLevels.length ? (
        <label>
          {zh ? '思考程度' : 'Thinking'}
          <select
            value={level ?? ''}
            disabled={busy}
            onChange={(event) => setThinking({ model: key(model), level: event.target.value })}
          >
            <option value="">{zh ? '默认' : 'Default'}</option>
            {model.thinkingLevels.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      {history.length > 1 ? (
        <Button
          label={t('Previous', '上一页', '上一頁')}
          isDisabled={busy}
          onClick={() => setHistory((items) => items.slice(0, -1))}
        />
      ) : null}
      {choices?.nextCursor ? (
        <Button
          label={t('More models', '更多模型', '更多模型')}
          isDisabled={busy}
          onClick={() => setHistory((items) => [...items, choices.nextCursor])}
        />
      ) : null}
      <Button
        label={t('Refresh', '刷新', '重新整理')}
        isDisabled={busy}
        onClick={() => {
          setHistory([null]);
          setRevision((value) => value + 1);
        }}
      />
      <Button
        label={label}
        isDisabled={busy || !model}
        onClick={() => {
          if (!model) return;
          setBusy(true);
          setError(undefined);
          void onSelect({ kind: 'model', model: model.model, thinkingLevel: level ?? null })
            .catch((error: unknown) =>
              setError(error instanceof Error ? error.message : String(error)),
            )
            .finally(() => setBusy(false));
        }}
      />
      {error ? <p role="alert">{error}</p> : null}
    </div>
  );
}
