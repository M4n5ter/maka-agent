import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, test } from 'node:test';
import type { BackendKind, SessionEvent, SessionHeader } from '@maka/core';
import type { BackendSendInput, PermissionDecision } from '@maka/core/backend-types';
import { BackendRegistry, type AgentBackend, type BackendFactoryContext, type SessionStore } from '@maka/runtime';
import type { Config } from '../contracts.js';
import type { HeadlessBackendContext, IsolatedToolExecutor } from '../isolation.js';
import {
  buildAiSdkCellBackendRegistration,
  buildHarborCellAiSdkTools,
  HARBOR_CELL_OUTPUT_FILENAME,
  HARBOR_CELL_RUNTIME_EVENTS_FILENAME,
  resolveHarborCellAiSdkEnv,
  runHarborCellFromEnv,
  runHarborCell,
} from '../harbor-cell.js';

const config: Config = {
  id: 'cell-cfg',
  backend: 'fake',
  llmConnectionSlug: 'fake',
  model: 'fake-model',
  systemPrompt: 'You are a benchmark cell agent.',
};

class CellReportingBackend implements AgentBackend {
  readonly sessionId: string;

  constructor(
    private readonly ctx: { sessionId: string; header: SessionHeader; store: SessionStore },
    readonly kind: BackendKind = 'fake',
  ) {
    this.sessionId = ctx.sessionId;
  }

  async *send(input: BackendSendInput): AsyncIterable<SessionEvent> {
    const ts = Date.now();
    await writeFile(join(this.ctx.header.cwd, 'cell-proof.txt'), 'ran in place\n', 'utf8');
    yield {
      type: 'token_usage',
      id: 'cell-usage',
      turnId: input.turnId,
      ts,
      input: 11,
      output: 7,
      total: 18,
      costUsd: 0.0042,
      systemPromptHash: 'sha256:cell-prompt',
    };
    yield { type: 'complete', id: 'cell-complete', turnId: input.turnId, ts, stopReason: 'end_turn' };
  }

  async stop(): Promise<void> {}
  async respondToPermission(_decision: PermissionDecision): Promise<void> {}
  async dispose(): Promise<void> {}
}

const registerCellBackend = (registry: BackendRegistry): void => {
  registry.register('fake', (ctx) =>
    new CellReportingBackend({ sessionId: ctx.sessionId, header: ctx.header, store: ctx.store }),
  );
};

class ThrowingBackend implements AgentBackend {
  readonly kind: BackendKind = 'fake';
  readonly sessionId: string;

  constructor(private readonly ctx: { sessionId: string }) {
    this.sessionId = ctx.sessionId;
  }

  async *send(_input: BackendSendInput): AsyncIterable<SessionEvent> {
    throw new Error('backend boom');
  }

  async stop(): Promise<void> {}
  async respondToPermission(_decision: PermissionDecision): Promise<void> {}
  async dispose(): Promise<void> {}
}

const registerThrowingBackend = (registry: BackendRegistry): void => {
  registry.register('fake', (ctx) => new ThrowingBackend({ sessionId: ctx.sessionId }));
};

describe('runHarborCell', () => {
  test('runs in the provided workspace and writes the shared cell artifacts', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const result = await runHarborCell({
        config,
        instruction: 'write the answer in-place',
        cwd: workspaceDir,
        outputDir,
        storageRoot,
        registerBackends: registerCellBackend,
      });

      assert.equal(await readFile(join(workspaceDir, 'cell-proof.txt'), 'utf8'), 'ran in place\n');
      assert.equal(result.output.status, 'completed');
      assert.equal(result.output.promptHash, 'sha256:cell-prompt');
      assert.equal(result.output.runtimeEventsPath, join(outputDir, HARBOR_CELL_RUNTIME_EVENTS_FILENAME));
      assert.equal(result.output.tokenSummary.costUsd, 0.0042);

      const outputJson = JSON.parse(await readFile(join(outputDir, HARBOR_CELL_OUTPUT_FILENAME), 'utf8'));
      assert.deepEqual(outputJson, result.output);
      const runtimeEvents = await readFile(join(outputDir, HARBOR_CELL_RUNTIME_EVENTS_FILENAME), 'utf8');
      assert.match(runtimeEvents, /"id":"cell-usage"/);
      assert.match(runtimeEvents, /"systemPromptHash":"sha256:cell-prompt"/);
    });
  });

  test('env entrypoint reads instruction files and writes the same cell artifacts', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const instructionFile = join(outputDir, 'instruction.txt');
      await writeFile(instructionFile, 'solve from env\n', 'utf8');

      const result = await runHarborCellFromEnv({
        MAKA_BACKEND: 'fake',
        MAKA_INSTRUCTION_FILE: instructionFile,
        MAKA_WORKDIR: workspaceDir,
        MAKA_OUTPUT_DIR: outputDir,
        MAKA_STORAGE_ROOT: storageRoot,
        MAKA_SYSTEM_PROMPT: config.systemPrompt!,
      }, {
        registerBackends: registerCellBackend,
      });

      assert.equal(result.output.status, 'completed');
      assert.equal(await readFile(join(workspaceDir, 'cell-proof.txt'), 'utf8'), 'ran in place\n');
      assert.deepEqual(
        JSON.parse(await readFile(join(outputDir, HARBOR_CELL_OUTPUT_FILENAME), 'utf8')),
        result.output,
      );
    });
  });

  test('env entrypoint defaults to the process cwd when MAKA_WORKDIR is absent', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const instructionFile = join(outputDir, 'instruction.txt');
      await writeFile(instructionFile, 'solve from current cwd\n', 'utf8');

      const originalCwd = process.cwd();
      process.chdir(workspaceDir);
      try {
        const result = await runHarborCellFromEnv({
          MAKA_BACKEND: 'fake',
          MAKA_INSTRUCTION_FILE: instructionFile,
          MAKA_OUTPUT_DIR: outputDir,
          MAKA_STORAGE_ROOT: storageRoot,
          MAKA_SYSTEM_PROMPT: config.systemPrompt!,
        }, {
          registerBackends: registerCellBackend,
        });

        assert.equal(result.output.status, 'completed');
        assert.equal(await readFile(join(workspaceDir, 'cell-proof.txt'), 'utf8'), 'ran in place\n');
      } finally {
        process.chdir(originalCwd);
      }
    });
  });

  test('writes a failed cell artifact when the backend stream throws', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const result = await runHarborCell({
        config,
        instruction: 'trigger backend failure',
        cwd: workspaceDir,
        outputDir,
        storageRoot,
        registerBackends: registerThrowingBackend,
      });

      assert.equal(result.output.status, 'failed');
      assert.equal(result.output.errorClass, 'Error');
      assert.match(
        await readFile(join(outputDir, HARBOR_CELL_OUTPUT_FILENAME), 'utf8'),
        /"status": "failed"/,
      );
    });
  });

  test('env entrypoint maps provider/model env for the real backend path', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const seenContexts: HeadlessBackendContext[] = [];
      const registerAiSdkBackend = (registry: BackendRegistry, context: HeadlessBackendContext): void => {
        seenContexts.push(context);
        registry.register('ai-sdk', (ctx) =>
          new CellReportingBackend({ sessionId: ctx.sessionId, header: ctx.header, store: ctx.store }, 'ai-sdk'),
        );
      };

      const result = await runHarborCellFromEnv({
        MAKA_INSTRUCTION: 'solve from real-provider env',
        MAKA_MODEL: 'openai/gpt-4o-mini',
        MAKA_WORKDIR: workspaceDir,
        MAKA_OUTPUT_DIR: outputDir,
        MAKA_STORAGE_ROOT: storageRoot,
        MAKA_SYSTEM_PROMPT: 'Use the benchmark prompt.',
      }, {
        registerBackends: registerAiSdkBackend,
      });

      assert.equal(result.output.status, 'completed');
      assert.equal(seenContexts.length, 1);
      assert.equal(seenContexts[0].config.backend, 'ai-sdk');
      assert.equal(seenContexts[0].config.llmConnectionSlug, 'openai');
      assert.equal(seenContexts[0].config.model, 'gpt-4o-mini');
      assert.equal(seenContexts[0].config.systemPrompt, 'Use the benchmark prompt.');
      assert.equal(seenContexts[0].realBackendIsolation?.kind, 'external');
      assert.equal(seenContexts[0].realBackendIsolation?.label, 'Harbor task container');
      assert.equal(typeof seenContexts[0].realBackendIsolation?.toolExecutor?.exec, 'function');
      assert.equal(typeof seenContexts[0].toolExecutor?.exec, 'function');
    });
  });

  test('Harbor ai-sdk backend registration exposes native file tools to the provider schema', async () => {
    await withDirs(async ({ workspaceDir }) => {
      const registry = new BackendRegistry();
      const toolExecutor = fakeToolExecutor();
      const register = buildAiSdkCellBackendRegistration({
        provider: 'openai',
        model: 'gpt-4o-mini',
        env: { OPENAI_API_KEY: 'test-key' },
        now: () => 123,
        newId: () => 'id',
      });
      await register(registry, {
        config: {
          id: 'harbor-ai-sdk',
          backend: 'ai-sdk',
          llmConnectionSlug: 'openai',
          model: 'gpt-4o-mini',
        },
        task: { id: 'harbor-cell', instruction: 'solve', workspaceDir },
        workspaceDir,
        realBackendIsolation: { kind: 'external', label: 'Harbor task container', toolExecutor },
        toolExecutor,
      });

      const backend = await registry.build('ai-sdk', backendContext(workspaceDir));
      const backendInput = (backend as unknown as {
        input: {
          tools: Array<{ name: string; permissionRequired?: boolean }>;
          systemPrompt?: string;
        };
      }).input;
      const toolNames = backendInput.tools.map((tool) => tool.name);

      for (const expected of ['Bash', 'Read', 'Write', 'Edit', 'Glob', 'Grep']) {
        assert.ok(toolNames.includes(expected), `expected provider schema tool ${expected}`);
      }
      assert.equal(backendInput.tools.find((tool) => tool.name === 'Bash')?.permissionRequired, false);
      assert.equal(backendInput.tools.find((tool) => tool.name === 'Write')?.permissionRequired, false);
      assert.match(backendInput.systemPrompt ?? '', /Prefer Read, Glob, and Grep/);
    });
  });

  test('Harbor tool builder keeps the six container-native tools non-interactive', () => {
    const tools = buildHarborCellAiSdkTools(fakeToolExecutor());
    const names = tools.map((tool) => tool.name);

    for (const expected of ['Bash', 'Read', 'Write', 'Edit', 'Glob', 'Grep']) {
      assert.ok(names.includes(expected), `expected Harbor tool ${expected}`);
      assert.equal(tools.find((tool) => tool.name === expected)?.permissionRequired, false);
    }
  });

  test('env entrypoint keeps slashful model ids when provider is explicit', async () => {
    await withDirs(async ({ workspaceDir, outputDir, storageRoot }) => {
      const seenContexts: HeadlessBackendContext[] = [];
      const registerAiSdkBackend = (registry: BackendRegistry, context: HeadlessBackendContext): void => {
        seenContexts.push(context);
        registry.register('ai-sdk', (ctx) =>
          new CellReportingBackend({ sessionId: ctx.sessionId, header: ctx.header, store: ctx.store }, 'ai-sdk'),
        );
      };

      await runHarborCellFromEnv({
        MAKA_INSTRUCTION: 'solve through an OpenAI-compatible gateway',
        MAKA_PROVIDER: 'openai-compatible',
        MAKA_MODEL: 'anthropic/claude-sonnet-4-5',
        MAKA_WORKDIR: workspaceDir,
        MAKA_OUTPUT_DIR: outputDir,
        MAKA_STORAGE_ROOT: storageRoot,
      }, {
        registerBackends: registerAiSdkBackend,
      });

      assert.equal(seenContexts[0].config.llmConnectionSlug, 'openai-compatible');
      assert.equal(seenContexts[0].config.model, 'anthropic/claude-sonnet-4-5');
    });
  });

  test('resolves ai-sdk connection env without constructing a network backend', () => {
    const gateway = resolveHarborCellAiSdkEnv({
      provider: 'openai-compatible',
      model: 'anthropic/claude-sonnet-4-5',
      env: {
        OPENAI_API_KEY: 'gateway-key',
        OPENAI_BASE_URL: 'https://gateway.example/v1',
      },
      ts: 123,
    });
    assert.equal(gateway.apiKey, 'gateway-key');
    assert.equal(gateway.connection.providerType, 'openai-compatible');
    assert.equal(gateway.connection.baseUrl, 'https://gateway.example/v1');
    assert.equal(gateway.connection.defaultModel, 'anthropic/claude-sonnet-4-5');

    const deepseek = resolveHarborCellAiSdkEnv({
      provider: 'deepseek',
      model: 'deepseek-chat',
      env: {
        OPENAI_API_KEY: 'fallback-key',
        OPENAI_BASE_URL: 'https://fallback.example/v1',
      },
      ts: 456,
    });
    assert.equal(deepseek.apiKey, 'fallback-key');
    assert.equal(deepseek.connection.baseUrl, 'https://fallback.example/v1');
  });
});

function fakeToolExecutor(): IsolatedToolExecutor {
  return {
    async exec() {
      return { exitCode: 0, stdout: '', stderr: '' };
    },
  };
}

function backendContext(workspaceDir: string): BackendFactoryContext {
  return {
    sessionId: 'session-1',
    workspaceRoot: workspaceDir,
    header: {
      id: 'session-1',
      cwd: workspaceDir,
      workspaceRoot: workspaceDir,
      createdAt: 123,
      lastUsedAt: 123,
      name: 'harbor cell test',
      isFlagged: false,
      labels: [],
      isArchived: false,
      status: 'active',
      hasUnread: false,
      backend: 'ai-sdk',
      llmConnectionSlug: 'openai',
      connectionLocked: true,
      model: 'gpt-4o-mini',
      permissionMode: 'execute',
      schemaVersion: 1,
    },
    store: {
      appendMessage: async () => {},
    } as unknown as SessionStore,
  };
}

async function withDirs<T>(
  fn: (dirs: { workspaceDir: string; outputDir: string; storageRoot: string }) => Promise<T>,
): Promise<T> {
  const workspaceDir = await mkdtemp(join(tmpdir(), 'maka-cell-ws-'));
  const outputDir = await mkdtemp(join(tmpdir(), 'maka-cell-out-'));
  const storageRoot = await mkdtemp(join(tmpdir(), 'maka-cell-store-'));
  try {
    return await fn({ workspaceDir, outputDir, storageRoot });
  } finally {
    await rm(workspaceDir, { recursive: true, force: true });
    await rm(outputDir, { recursive: true, force: true });
    await rm(storageRoot, { recursive: true, force: true });
  }
}
