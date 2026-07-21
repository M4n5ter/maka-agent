import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { stableLocalMemoryEntryId } from '@maka/core/local-memory';
import {
  buildBuiltinTools,
  evaluateGoal,
  GoalEvaluatorFatalError,
  parseOAuthSubscriptionTokens,
} from '@maka/runtime';
import { openInteractiveExecutionStoresForWrite } from '@maka/storage/execution-stores';
import { openInteractiveMemoryStoreForWrite } from '@maka/storage/memory-store';
import { openInteractiveRuntimePolicyStoresForWrite } from '@maka/storage/runtime-policy-stores';
import { resolveStorageRoot, tryAcquireInteractiveRootOwner } from '@maka/storage/root-authority';
import { openInteractiveTaskLedgerStoreForWrite } from '@maka/storage/task-ledger-store';
import {
  createHostExecutionModelComposition,
  createHostGoalEvaluator,
} from '../server/execution-model-composition.js';
import { HostGoalCoordinator } from '../server/goal-coordinator.js';
import { RuntimeHostKernel } from '../server/host-kernel.js';
import { HostMemoryCoordinator } from '../server/memory-coordinator.js';
import { createUnavailableDomainOperationHandlers } from '../server/operation-dispatcher.js';
import { HostSkillCatalogCoordinator } from '../server/skill-catalog-coordinator.js';
import { HostSkillCatalogFilesystem } from '../server/skill-catalog-filesystem.js';

const SESSION_ID = 'model-composition-session';
const NOW = 1_700_000_000_000;

test('Goal post-cut work commits before fail-stop owner isolation and releases residency after it', async () => {
  const fixture = await createGoalOAuthFixture();
  const refresh = deferred<{
    access_token: string;
    refresh_token: string;
    expires_at: number;
  }>();
  const invalidationEntered = deferred<void>();
  const releaseInvalidation = deferred<void>();
  let evaluator!: ReturnType<typeof createHostGoalEvaluator>;
  let requestFailStop!: Parameters<
    NonNullable<Parameters<typeof RuntimeHostKernel.start>[0]['compositionFactory']>
  >[0]['requestFailStop'];
  let releases = 0;
  const isolationCause = new Error('controlled Goal fail-stop');
  try {
    const host = await RuntimeHostKernel.start({
      owner: fixture.owner,
      idleGraceMs: 10_000,
      compositionFactory: async (context) => {
        requestFailStop = context.requestFailStop;
        evaluator = createHostGoalEvaluator({
          sessions: fixture.executionStores.sessionStore,
          runtimePolicy: fixture.policyStores,
          acquireResidency: () => {
            const residency = context.acquireResidency();
            return {
              release: () => {
                releases += 1;
                residency.release();
              },
            };
          },
          onCredentialRefreshed: async () => {
            invalidationEntered.resolve();
            await releaseInvalidation.promise;
          },
          onFatal: (error) =>
            assert.fail(`successful post-cut work became fatal: ${error.message}`),
          refreshOAuthTokens: () => {
            fixture.refreshBegan.resolve();
            return refresh.promise;
          },
        });
        return {
          handlers: createUnavailableDomainOperationHandlers(),
          beginDrain: () => evaluator.beginDrain(),
          recover: async () => undefined,
          close: async () => {
            await evaluator.close();
            return { kind: 'clean' };
          },
        };
      },
    });
    const controller = new AbortController();
    const evaluation = evaluator.evaluate('prompt', fixture.sessionId, controller.signal);
    await fixture.refreshBegan.promise;
    controller.abort(new Error('Goal evaluator timed out'));
    const failStop = evaluator.prepareFailStop();
    requestFailStop({ kind: 'fail_stop', cause: isolationCause, ...failStop });

    assert.equal(fixture.owner.closed, false);
    assert.equal(await tryAcquireInteractiveRootOwner(fixture.capability), undefined);
    refresh.resolve({
      access_token: 'new-access',
      refresh_token: 'new-refresh',
      expires_at: Date.now() + 60_000,
    });
    await invalidationEntered.promise;
    const resolved = await fixture.policyStores.operations.resolveExecutionConnection('goal-oauth');
    assert.equal(resolved.kind, 'ready');
    if (resolved.kind === 'ready') {
      assert.equal(
        parseOAuthSubscriptionTokens(resolved.secretMaterial.connection?.secret ?? '')
          ?.access_token,
        'new-access',
      );
    }
    assert.equal(fixture.owner.closed, false);
    assert.equal(releases, 0);

    releaseInvalidation.resolve();
    await assert.rejects(evaluation, /timed out/);
    await assert.rejects(host.closed, (error: unknown) => error === isolationCause);
    assert.equal(fixture.owner.closed, true);
    assert.equal(releases, 1);
    const successor = await tryAcquireInteractiveRootOwner(fixture.capability);
    assert.ok(successor);
    await successor?.close();
  } finally {
    refresh.reject(new Error('test cleanup'));
    releaseInvalidation.resolve();
    await fixture.owner.close().catch(() => undefined);
    await rm(fixture.base, { recursive: true, force: true });
  }
});

test('Goal post-cut failure fail-stops the Host after evaluateGoal already timed out', async () => {
  const fixture = await createGoalOAuthFixture();
  const refreshFailure = new Error('remote OAuth refresh failed');
  const releaseRefreshFailure = deferred<void>();
  let callbackFatal: GoalEvaluatorFatalError | undefined;
  let evaluator!: ReturnType<typeof createHostGoalEvaluator>;
  let goals!: HostGoalCoordinator;
  let rootAdmissions = 0;
  const continuationEvaluated = deferred<void>();
  const admissionFenceEntered = deferred<void>();
  let continuationVerdict = '';
  try {
    const host = await RuntimeHostKernel.start({
      owner: fixture.owner,
      idleGraceMs: 10_000,
      compositionFactory: async (context) => {
        evaluator = createHostGoalEvaluator({
          sessions: fixture.executionStores.sessionStore,
          runtimePolicy: fixture.policyStores,
          acquireResidency: context.acquireResidency,
          onCredentialRefreshed: async () => undefined,
          refreshOAuthTokens: async () => {
            fixture.refreshBegan.resolve();
            await releaseRefreshFailure.promise;
            throw refreshFailure;
          },
          onFatal: (fatal) => {
            callbackFatal = fatal;
            goals.beginDrain();
            const failStop = evaluator.prepareFailStop();
            context.requestFailStop({ kind: 'fail_stop', cause: fatal, ...failStop });
          },
        });
        goals = new HostGoalCoordinator({
          root: {
            admitGoalTurn: async () => {
              rootAdmissions++;
              return { kind: 'unavailable', reason: 'test root stopped' };
            },
          },
          evaluate: async () => {
            continuationEvaluated.resolve();
            return continuationVerdict;
          },
          waitForEvaluatorPostCutEffects: () => {
            admissionFenceEntered.resolve();
            return evaluator.whenCurrentPostCutEffectsSettled();
          },
          readEvaluationContext: async () => ({ recentContext: '', tokenCount: 0 }),
          acquireResidency: context.acquireResidency,
          requestDrain: () => undefined,
        });
        return {
          handlers: createUnavailableDomainOperationHandlers(),
          beginDrain: () => {
            goals.beginDrain();
            evaluator.beginDrain();
          },
          recover: async () => undefined,
          close: async () => {
            await Promise.all([goals.close(), evaluator.close()]);
            return { kind: 'clean' };
          },
        };
      },
    });
    let triggerEvaluatorTimeout: (() => void) | undefined;
    let timerRegistrations = 0;
    const verdict = evaluateGoal(
      {
        ...evaluator,
        timeoutMs: 5,
        abortCleanupGraceMs: 5,
        setTimeout: (callback) => {
          timerRegistrations += 1;
          if (timerRegistrations === 1) triggerEvaluatorTimeout = callback;
          else queueMicrotask(callback);
          return callback;
        },
        clearTimeout: () => undefined,
      },
      'finish',
      'context',
      fixture.sessionId,
    );
    await fixture.refreshBegan.promise;
    assert.ok(triggerEvaluatorTimeout);
    triggerEvaluatorTimeout();
    const timedOut = await verdict;
    assert.equal(timedOut.evaluatorFailed, true);
    assert.match(timedOut.reason, /timed out/);
    assert.equal(callbackFatal, undefined);
    continuationVerdict = JSON.stringify({
      met: timedOut.met,
      impossible: timedOut.impossible,
      progress: timedOut.progress,
      waiting: timedOut.waiting,
      reason: timedOut.reason,
    });
    goals.manager.create(fixture.sessionId, 'continue after evaluator timeout');
    const turn = goals.beginExternalTurn(fixture.sessionId, 'timed-out-goal-turn');
    assert.equal(turn.kind, 'registered');
    if (turn.kind !== 'registered') throw new Error('Goal turn registration failed');
    const settling = turn.settle({ kind: 'completed', turnId: 'timed-out-goal-turn' });
    await continuationEvaluated.promise;
    await admissionFenceEntered.promise;
    assert.equal(rootAdmissions, 0);

    let fenceSettled = false;
    const fence = evaluator.whenCurrentPostCutEffectsSettled().then(() => {
      fenceSettled = true;
    });
    await Promise.resolve();
    assert.equal(fenceSettled, false);
    releaseRefreshFailure.resolve();
    await assert.rejects(host.closed, (error: unknown) => {
      assert.ok(callbackFatal, 'detached evaluator failure did not synchronously call fail-stop');
      return error === callbackFatal && callbackFatal.fatalCause === refreshFailure;
    });
    await Promise.all([settling, fence]);
    assert.equal(fenceSettled, true);
    assert.equal(rootAdmissions, 0);
  } finally {
    releaseRefreshFailure.resolve();
    await fixture.owner.close().catch(() => undefined);
    await rm(fixture.base, { recursive: true, force: true });
  }
});

test('composes only canonical Host model context in the fixed order', async () => {
  const base = await mkdtemp(join(tmpdir(), 'maka-host-model-composition-'));
  const root = join(base, 'root');
  const managedSources = join(base, 'managed-sources');
  const capability = await resolveStorageRoot({ path: root, kind: 'interactive' });
  const owner = await tryAcquireInteractiveRootOwner(capability);
  assert.ok(owner);
  if (!owner) return;

  const policyStores = await openInteractiveRuntimePolicyStoresForWrite(owner.lease);
  const memoryStore = await openInteractiveMemoryStoreForWrite(owner.lease);
  const taskLedger = await openInteractiveTaskLedgerStoreForWrite(owner.lease);
  const skills = new HostSkillCatalogCoordinator(
    new HostSkillCatalogFilesystem(owner.lease, managedSources),
  );
  const memory = new HostMemoryCoordinator(memoryStore, policyStores.runtimePolicy, () => NOW);

  try {
    await mkdir(join(root, 'skills', 'canonical-skill'), { recursive: true });
    await writeFile(
      join(root, 'skills', 'canonical-skill', 'SKILL.md'),
      skillDocument('Canonical Skill', 'Read from the Host snapshot', 'FIRST_BODY', ['Bash']),
      'utf8',
    );
    await writeFile(join(root, 'AGENTS.md'), 'WORKSPACE_INSTRUCTION_SENTINEL\n', 'utf8');
    await skills.recover();

    const recoveredScan = skills.readCanonicalModelSkills();
    assert.equal(recoveredScan.length, 1);
    assert.equal(recoveredScan[0]?.content.trim(), 'FIRST_BODY');
    assert.equal(recoveredScan[0]?.content.includes('name: Canonical Skill'), false);
    assert.equal(Object.isFrozen(recoveredScan), true);
    assert.equal(Object.isFrozen(recoveredScan[0]), true);

    await writeFile(
      join(root, 'skills', 'canonical-skill', 'SKILL.md'),
      skillDocument('Canonical Skill', 'Changed outside the snapshot', 'SECOND_BODY', ['Bash']),
      'utf8',
    );
    assert.equal(skills.readCanonicalModelSkills()[0]?.content.trim(), 'FIRST_BODY');

    const memoryContent = memoryDocument('MEMORY_SENTINEL');
    await memoryStore.save({ expectedRevision: null, bytes: Buffer.from(memoryContent) });

    const initialPolicy = await policyStores.runtimePolicy.getSnapshot();
    assert.equal(await memory.readCanonicalModelPrompt(initialPolicy.policy), undefined);
    const personalized = await policyStores.runtimePolicy.mutate({
      expectedRevision: initialPolicy.revision,
      operation: {
        kind: 'set_personalization',
        value: { displayName: 'Canonical User', assistantTone: 'COMPOSITION_TONE_SENTINEL' },
      },
    });
    assert.equal(personalized.kind, 'committed');
    if (personalized.kind !== 'committed') return;
    const memoryEnabled = await policyStores.runtimePolicy.mutate({
      expectedRevision: personalized.snapshot.revision,
      operation: { kind: 'set_memory', value: { enabled: true, agentReadEnabled: true } },
    });
    assert.equal(memoryEnabled.kind, 'committed');
    if (memoryEnabled.kind !== 'committed') return;

    await taskLedger.create(SESSION_ID, [{ subject: 'TASK_LEDGER_SENTINEL' }]);

    const bashTool = buildBuiltinTools().find((tool) => tool.name === 'Bash');
    assert.ok(bashTool);
    const composition = createHostExecutionModelComposition({
      policy: policyStores.runtimePolicy,
      skills,
      memory,
      taskLedger,
      runtimeTools: [bashTool],
      platform: 'linux',
      shell: 'test-shell',
      now: () => new Date('2026-07-21T00:00:00Z'),
    });
    assert.deepEqual(
      composition.tools.map((tool) => tool.name),
      ['AskUserQuestion', 'Skill', 'task_create', 'task_update', 'task_list', 'task_get', 'Bash'],
    );

    const context = { sessionId: SESSION_ID, cwd: root };
    const system = await composition.systemPrompt(context);
    assert.ok(system);
    assertOrdered(system, [
      'COMPOSITION_TONE_SENTINEL',
      'Canonical Skill',
      'WORKSPACE_INSTRUCTION_SENTINEL',
      'MEMORY_SENTINEL',
    ]);
    assert.equal(system.includes('SECOND_BODY'), false);

    await skills.recover();
    const loaded = (await composition.tools[1]!.impl(
      { name: 'canonical-skill' },
      {
        sessionId: SESSION_ID,
        turnId: 'model-composition-turn',
        cwd: root,
        toolCallId: 'skill-call',
        abortSignal: new AbortController().signal,
        emitOutput: () => undefined,
      },
    )) as { readonly ok: boolean; readonly skill?: { readonly instructions: string } };
    assert.equal(loaded.ok, true);
    assert.equal(loaded.skill?.instructions.trim(), 'SECOND_BODY');

    const tail = await composition.turnTailPrompt(context);
    assertOrdered(tail, ['Maka session environment', 'TASK_LEDGER_SENTINEL']);
    assert.equal(tail.includes('<local-memory>'), false);

    const latestPolicy = await policyStores.runtimePolicy.getSnapshot();
    const disabled = await policyStores.runtimePolicy.mutate({
      expectedRevision: latestPolicy.revision,
      operation: { kind: 'set_memory', value: { enabled: false, agentReadEnabled: true } },
    });
    assert.equal(disabled.kind, 'committed');
    if (disabled.kind !== 'committed') return;
    assert.equal(await memory.readCanonicalModelPrompt(disabled.snapshot.policy), undefined);
    const reenabled = await policyStores.runtimePolicy.mutate({
      expectedRevision: disabled.snapshot.revision,
      operation: { kind: 'set_memory', value: { enabled: true, agentReadEnabled: true } },
    });
    assert.equal(reenabled.kind, 'committed');
    if (reenabled.kind !== 'committed') return;
    const incognito = await policyStores.runtimePolicy.mutate({
      expectedRevision: reenabled.snapshot.revision,
      operation: { kind: 'set_privacy', value: { incognitoActive: true } },
    });
    assert.equal(incognito.kind, 'committed');
    if (incognito.kind !== 'committed') return;
    assert.equal(await memory.readCanonicalModelPrompt(incognito.snapshot.policy), undefined);

    const visibleAgain = await policyStores.runtimePolicy.mutate({
      expectedRevision: incognito.snapshot.revision,
      operation: { kind: 'set_privacy', value: { incognitoActive: false } },
    });
    assert.equal(visibleAgain.kind, 'committed');
    if (visibleAgain.kind !== 'committed') return;
    assert.ok(
      (await memory.readCanonicalModelPrompt(visibleAgain.snapshot.policy))?.includes(
        'MEMORY_SENTINEL',
      ),
    );

    const advancingPolicy = {
      getSnapshot: async () => {
        const snapshot = await policyStores.runtimePolicy.getSnapshot();
        if (!snapshot.policy.privacy.incognitoActive) {
          const advanced = await policyStores.runtimePolicy.mutate({
            expectedRevision: snapshot.revision,
            operation: { kind: 'set_privacy', value: { incognitoActive: true } },
          });
          assert.equal(advanced.kind, 'committed');
        }
        return snapshot;
      },
    };
    const coherentComposition = createHostExecutionModelComposition({
      policy: advancingPolicy,
      skills,
      memory,
      taskLedger,
      runtimeTools: [bashTool],
    });
    const coherentSystem = await coherentComposition.systemPrompt(context);
    assert.ok(coherentSystem);
    assert.ok(coherentSystem.includes('COMPOSITION_TONE_SENTINEL'));
    assert.ok(coherentSystem.includes('MEMORY_SENTINEL'));
    assert.equal(
      (await policyStores.runtimePolicy.getSnapshot()).policy.privacy.incognitoActive,
      true,
    );
  } finally {
    skills.beginDrain();
    await skills.close();
    await memoryStore.beginDrain();
    await memoryStore.close();
    owner.beginClose();
    await owner.close();
    await rm(base, { recursive: true, force: true });
  }
});

async function createGoalOAuthFixture() {
  const base = await mkdtemp(join(tmpdir(), 'maka-host-goal-evaluator-'));
  const root = join(base, 'root');
  const capability = await resolveStorageRoot({ path: root, kind: 'interactive' });
  const owner = await tryAcquireInteractiveRootOwner(capability);
  assert.ok(owner);
  if (!owner) throw new Error('Unable to acquire Goal evaluator test root');
  try {
    const executionStores = await openInteractiveExecutionStoresForWrite(owner.lease);
    const policyStores = await openInteractiveRuntimePolicyStoresForWrite(owner.lease);
    const created = await policyStores.connectionCatalog.create({
      expectedCatalogRevision: 0,
      connection: {
        slug: 'goal-oauth',
        name: 'Goal OAuth',
        providerType: 'claude-subscription',
        enabled: true,
        enabledModelIds: ['goal-model'],
      },
    });
    assert.equal(created.kind, 'committed');
    if (created.kind !== 'committed') throw new Error('Goal OAuth connection was not created');
    const connection = created.snapshot.connections[0];
    assert.ok(connection);
    if (!connection) throw new Error('Goal OAuth connection is missing');
    const credential = await policyStores.credentialVault.set({
      locator: {
        scope: 'connection',
        connectionId: connection.connectionId,
        kind: 'oauth_token',
      },
      expected: null,
      secret: JSON.stringify({
        access_token: 'old-access',
        refresh_token: 'old-refresh',
        expires_at: 1,
      }),
    });
    assert.equal(credential.kind, 'committed');
    const modelFetch = await policyStores.operations.beginModelFetch(connection.connectionId);
    assert.equal(modelFetch.kind, 'ready');
    if (modelFetch.kind !== 'ready') throw new Error('Goal OAuth model fetch was not admitted');
    const modelCommit = await policyStores.operations.completeModelFetch(modelFetch.ticket, {
      models: [
        {
          id: 'goal-model',
          capabilities: { chat: true, functionCalling: true },
        },
      ],
      source: 'fetched',
      fetchedAt: Date.now(),
    });
    assert.equal(modelCommit.kind, 'committed');
    const session = await executionStores.sessionStore.create({
      cwd: root,
      backend: 'ai-sdk',
      llmConnectionSlug: 'goal-oauth',
      model: 'goal-model',
      permissionMode: 'ask',
    });
    return {
      base,
      capability,
      owner,
      executionStores,
      policyStores,
      sessionId: session.id,
      refreshBegan: deferred<void>(),
    };
  } catch (error) {
    await owner.close().catch(() => undefined);
    await rm(base, { recursive: true, force: true });
    throw error;
  }
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function skillDocument(
  name: string,
  description: string,
  body: string,
  requiredTools: readonly string[] = [],
): string {
  const requirements =
    requiredTools.length > 0 ? `required-tools: [${requiredTools.join(', ')}]\n` : '';
  return `---\nname: ${name}\ndescription: ${description}\n${requirements}---\n${body}\n`;
}

function memoryDocument(content: string): string {
  const id = stableLocalMemoryEntryId(content, NOW);
  return [
    '# Maka Memory',
    '',
    '## Canonical preference',
    `<!-- maka-memory: id=${id} origin=manual createdAt=${NOW} status=active -->`,
    content,
    '',
  ].join('\n');
}

function assertOrdered(source: string, sentinels: readonly string[]): void {
  let previous = -1;
  for (const sentinel of sentinels) {
    const current = source.indexOf(sentinel);
    assert.ok(current > previous, `${sentinel} must follow the previous fragment`);
    previous = current;
  }
}
