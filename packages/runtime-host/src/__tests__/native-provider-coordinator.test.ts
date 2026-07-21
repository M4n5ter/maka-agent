import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import {
  NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS,
  type NativeProviderClientFrame,
  type NativeProviderHostFrame,
  type NativeProviderResultPayload,
  type NativeProviderSessionReleaseFrame,
  type NativeProviderSubcallFrame,
} from '../protocol/index.js';
import {
  HostNativeProviderCoordinator,
  type HostNativeProviderInvocation,
  type NativeProviderConnectionAttachment,
} from '../server/native-provider-coordinator.js';

const CONTEXT = {
  hostEpoch: 'host-epoch',
  connectionId: 'unused',
  surface: 'desktop',
  principal: 'local_os_user',
  acquireResidency: () => ({ release() {} }),
} as const;

test('acquisition freezes one registration and affinity cannot migrate', async () => {
  const coordinator = createCoordinator();
  const first = attach(coordinator, 'first');
  const second = attach(coordinator, 'second');
  const firstRegistration = await register(coordinator, 'first');
  const invocation = acquire(coordinator, 'operation', firstRegistration);
  await register(coordinator, 'second');

  const ambiguous = coordinator.acquireInvocation({
    operationId: 'ambiguous',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(ambiguous.ok, false);
  if (!ambiguous.ok) assert.equal(ambiguous.error, 'capability_ambiguous');

  const pending = invocation.call({
    subcall: preflight('frozen'),
    signal: signal(),
  });
  const call = requireSubcall(first.sent[0]);
  assert.equal(second.sent.length, 0);
  first.attachment.accept(
    success(call, {
      kind: 'preflight',
      accessibility: true,
      screenRecording: true,
    }),
  );
  assert.equal((await pending).ok, true);
  invocation.release();

  const unregister = await coordinator.handlers['native.provider.unregister'](
    { registrationId: firstRegistration },
    { ...CONTEXT, connectionId: 'first' },
  );
  assert.equal(unregister.ok, true);
  const mismatch = coordinator.acquireInvocation({
    operationId: 'must-not-migrate',
    sessionId: 'session',
    capability: 'computer_use',
    affinity: firstRegistration,
  });
  assert.equal(mismatch.ok, false);
  if (!mismatch.ok) assert.equal(mismatch.error, 'service_mismatch');

  first.attachment.close();
  second.attachment.close();
  await coordinator.close();
});

test('subcalls are serial and successful admissions receive increasing ordinals', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'serial');

  const firstPending = invocation.call({
    subcall: preflight('first'),
    signal: signal(),
  });
  const first = requireSubcall(provider.sent[0]);
  const concurrent = await invocation.call({
    subcall: preflight('concurrent'),
    signal: signal(),
  });
  assert.equal(concurrent.ok, false);
  if (!concurrent.ok) assert.equal(concurrent.error.code, 'operation_failed');
  assert.equal(provider.sent.length, 1);
  provider.attachment.accept(success(first, preflightResult()));
  assert.equal((await firstPending).ok, true);

  const secondPending = invocation.call({
    subcall: preflight('second'),
    signal: signal(),
  });
  const second = requireSubcall(provider.sent[1]);
  assert.equal(first.ordinal, 1);
  assert.equal(second.ordinal, 2);
  assert.notEqual(first.subcallId, second.subcallId);
  provider.attachment.accept(success(second, preflightResult()));
  await secondPending;

  invocation.release();
  provider.attachment.close();
  await coordinator.close();
});

test('rejects a subcall whose Session differs from its acquired outer identity', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'session-mismatch');
  const subcall = preflight('session-mismatch');

  const outcome = await invocation.call({
    subcall: {
      ...subcall,
      context: { ...subcall.context, sessionId: 'different-session' },
    },
    signal: signal(),
  });
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.equal(outcome.error.code, 'operation_failed');
  assert.deepEqual(provider.sent, []);

  invocation.release();
  await coordinator.releaseSession('session');
  await coordinator.releaseSession('different-session');
  assert.deepEqual(provider.sent, []);
  provider.attachment.close();
  await coordinator.close();
});

test('abort after admission sends cancel and waits for provider terminal', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'cancel');
  const abort = new AbortController();
  let settled = false;

  const pending = invocation
    .call({ subcall: preflight('cancel'), signal: abort.signal })
    .finally(() => (settled = true));
  const call = requireSubcall(provider.sent[0]);
  abort.abort();
  assert.equal(provider.sent[1]?.kind, 'native.provider.cancel');
  await turn();
  assert.equal(provider.sent.filter((frame) => frame.kind === 'native.provider.cancel').length, 1);
  assert.equal(settled, false);

  provider.attachment.accept(failure(call, 'operation_failed'));
  const outcome = await pending;
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.equal(outcome.error.code, 'operation_failed');
  invocation.release();
  provider.attachment.close();
  await coordinator.close();
});

test('unregister leaves selection immediately but acknowledges after invocation release', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  const registrationId = await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'held');
  const pending = invocation.call({
    subcall: preflight('held'),
    signal: signal(),
  });
  const subcall = requireSubcall(provider.sent[0]);
  provider.attachment.accept(success(subcall, preflightResult()));
  await pending;
  let acknowledged = false;

  const unregister = coordinator.handlers['native.provider.unregister'](
    { registrationId },
    { ...CONTEXT, connectionId: 'provider' },
  ).finally(() => (acknowledged = true));
  await turn();
  assert.equal(acknowledged, false);
  const unavailable = coordinator.acquireInvocation({
    operationId: 'new',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(unavailable.ok, false);
  if (!unavailable.ok) assert.equal(unavailable.error, 'capability_unavailable');

  invocation.release();
  assert.equal((await unregister).ok, true);
  assert.equal(provider.sent.at(-1)?.kind, 'native.provider.release');
  provider.attachment.close();
  await coordinator.close();
});

test('host drain rejects new work while an admitted invocation may keep calling', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'during-drain');
  coordinator.beginDrain();

  const rejected = coordinator.acquireInvocation({
    operationId: 'after-drain',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(rejected.ok, false);
  const registration = await coordinator.handlers['native.provider.register'](
    { capabilities: ['computer_use'] },
    { ...CONTEXT, connectionId: 'provider' },
  );
  assert.equal(registration.ok, false);
  if (!registration.ok) assert.equal(registration.error.code, 'host_draining');

  const pending = invocation.call({
    subcall: preflight('existing'),
    signal: signal(),
  });
  const call = requireSubcall(provider.sent[0]);
  provider.attachment.accept(success(call, preflightResult()));
  assert.equal((await pending).ok, true);
  invocation.release();
  provider.attachment.close();
  await coordinator.close();
});

test('disconnect makes an admitted subcall unknown and future calls lost without fallback', async () => {
  let residencies = 0;
  const coordinator = new HostNativeProviderCoordinator('host-epoch', () => {
    residencies += 1;
    return { release: () => (residencies -= 1) };
  });
  const first = attach(coordinator, 'first');
  const second = attach(coordinator, 'second');
  const affinity = await register(coordinator, 'first');
  const invocation = acquire(coordinator, 'disconnect', affinity);
  await register(coordinator, 'second');
  const pending = invocation.call({
    subcall: preflight('disconnect'),
    signal: signal(),
  });
  assert.equal(first.sent.length, 1);

  first.attachment.close();
  const outcome = await pending;
  assert.equal(outcome.ok, false);
  if (!outcome.ok) assert.equal(outcome.error.code, 'outcome_unknown');
  const future = await invocation.call({
    subcall: preflight('future'),
    signal: signal(),
  });
  assert.equal(future.ok, false);
  if (!future.ok) assert.equal(future.error.code, 'capability_lost');
  assert.equal(second.sent.length, 0);
  assert.equal(residencies, 0);

  second.attachment.close();
  await coordinator.close();
});

test('enqueue throw is capability_lost but a post-admission disconnect is outcome_unknown', async () => {
  const before = createCoordinator();
  const throwing = attach(before, 'throwing', () => {
    throw new Error('closed writer');
  });
  await register(before, 'throwing');
  const notAdmitted = acquire(before, 'not-admitted');
  const lost = await notAdmitted.call({
    subcall: preflight('lost'),
    signal: signal(),
  });
  assert.equal(lost.ok, false);
  if (!lost.ok) assert.equal(lost.error.code, 'capability_lost');
  assert.doesNotThrow(() => notAdmitted.release());
  throwing.attachment.close();
  await before.close();

  const after = createCoordinator();
  let attachment: NativeProviderConnectionAttachment | undefined;
  attachment = after.attachConnection('reentrant', {
    enqueue: () => {
      attachment?.close();
      return { flushed: Promise.resolve() };
    },
    close() {},
  });
  await register(after, 'reentrant');
  const admitted = acquire(after, 'admitted');
  const unknown = await admitted.call({
    subcall: preflight('unknown'),
    signal: signal(),
  });
  assert.equal(unknown.ok, false);
  if (!unknown.ok) assert.equal(unknown.error.code, 'outcome_unknown');
  await after.close();
});

test('attachment bytes are ordered and hash-checked before exposure', async () => {
  let residencies = 0;
  const coordinator = new HostNativeProviderCoordinator('host-epoch', () => {
    residencies += 1;
    return { release: () => (residencies -= 1) };
  });
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'attachments');
  const bytes = Buffer.from('verified-native-screenshot');

  const validPending = invocation.call({
    subcall: capture('valid'),
    signal: signal(),
  });
  const valid = requireSubcall(provider.sent[0]);
  provider.attachment.accept(chunk(valid, 'image', 0, bytes.subarray(0, 8)));
  provider.attachment.accept(chunk(valid, 'image', 1, bytes.subarray(8)));
  provider.attachment.accept(success(valid, captureResult(bytes, digest(bytes))));
  const validOutcome = await validPending;
  assert.equal(validOutcome.ok, true);
  if (validOutcome.ok) assert.deepEqual(validOutcome.attachments[0]?.bytes, bytes);

  const invalidPending = invocation.call({
    subcall: capture('invalid'),
    signal: signal(),
  });
  const invalid = requireSubcall(provider.sent[1]);
  provider.attachment.accept(chunk(invalid, 'image', 0, bytes));
  provider.attachment.accept(success(invalid, captureResult(bytes, '0'.repeat(64))));
  const invalidOutcome = await invalidPending;
  assert.equal(invalidOutcome.ok, false);
  if (!invalidOutcome.ok) assert.equal(invalidOutcome.error.code, 'outcome_unknown');
  assert.equal(provider.isClosed(), true);
  assert.equal(residencies, 0);
  await coordinator.close();
});

test('invocation limit, release idempotence, and close all preserve residency accounting', async () => {
  let residencies = 0;
  const coordinator = new HostNativeProviderCoordinator('host-epoch', () => {
    residencies += 1;
    return { release: () => (residencies -= 1) };
  });
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocations = Array.from({ length: NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS }, (_, index) =>
    acquire(coordinator, `operation-${index}`),
  );
  assert.equal(residencies, NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS);
  const overflow = coordinator.acquireInvocation({
    operationId: 'overflow',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(overflow.ok, false);
  if (!overflow.ok) assert.equal(overflow.error, 'capability_unavailable');

  let closed = false;
  const closing = coordinator.close().finally(() => (closed = true));
  await turn();
  assert.equal(closed, false);
  for (const invocation of invocations) {
    assert.doesNotThrow(() => invocation.release());
    assert.doesNotThrow(() => invocation.release());
  }
  await closing;
  assert.equal(residencies, 0);
});

test('acknowledged Session release fences acquisition and forgets only the actual owner', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'owned');
  const pending = invocation.call({
    subcall: preflight('owned'),
    signal: signal(),
  });
  const subcall = requireSubcall(provider.sent[0]);
  provider.attachment.accept(success(subcall, preflightResult()));
  await pending;
  invocation.release();

  const releaseTask = coordinator.releaseSession('session');
  const release = requireSessionRelease(provider.sent.at(-1));
  const fenced = coordinator.acquireInvocation({
    operationId: 'fenced',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(fenced.ok, false);
  provider.attachment.accept({
    kind: 'native.provider.session_released',
    hostEpoch: release.hostEpoch,
    registrationId: release.registrationId,
    releaseId: release.releaseId,
    sessionId: release.sessionId,
  });
  await releaseTask;

  const reopened = acquire(coordinator, 'reopened');
  reopened.release();
  await coordinator.releaseSession('session');
  assert.equal(
    provider.sent.filter((frame) => frame.kind === 'native.provider.session_release').length,
    1,
  );
  provider.attachment.close();
  await coordinator.close();
});

test('wrong Session release acknowledgement closes the registration and releases its fence', async () => {
  const coordinator = createCoordinator();
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'owned-invalid-ack');
  const pending = invocation.call({
    subcall: preflight('owned-invalid-ack'),
    signal: signal(),
  });
  const subcall = requireSubcall(provider.sent[0]);
  provider.attachment.accept(success(subcall, preflightResult()));
  await pending;
  invocation.release();

  const releaseTask = coordinator.releaseSession('session');
  const release = requireSessionRelease(provider.sent.at(-1));
  provider.attachment.accept({
    kind: 'native.provider.session_released',
    hostEpoch: release.hostEpoch,
    registrationId: 'wrong-registration',
    releaseId: release.releaseId,
    sessionId: release.sessionId,
  });
  await releaseTask;
  assert.equal(provider.isClosed(), true);
  const unavailable = coordinator.acquireInvocation({
    operationId: 'after-invalid-ack',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(unavailable.ok, false);
  await coordinator.close();
});

test('Session release timeout evicts the transport before releasing its fence', async () => {
  const coordinator = new HostNativeProviderCoordinator('host-epoch', () => ({ release() {} }), {
    releaseTimeoutMs: 5,
  });
  const provider = attach(coordinator, 'provider');
  await register(coordinator, 'provider');
  const invocation = acquire(coordinator, 'owned-timeout');
  const pending = invocation.call({
    subcall: preflight('owned-timeout'),
    signal: signal(),
  });
  const subcall = requireSubcall(provider.sent[0]);
  provider.attachment.accept(success(subcall, preflightResult()));
  await pending;
  invocation.release();

  await coordinator.releaseSession('session');
  assert.equal(provider.isClosed(), true);
  const unavailable = coordinator.acquireInvocation({
    operationId: 'after-timeout',
    sessionId: 'session',
    capability: 'computer_use',
  });
  assert.equal(unavailable.ok, false);
  await coordinator.close();
});

function createCoordinator(): HostNativeProviderCoordinator {
  return new HostNativeProviderCoordinator('host-epoch', () => ({
    release() {},
  }));
}

function attach(
  coordinator: HostNativeProviderCoordinator,
  connectionId: string,
  enqueue: (frame: NativeProviderHostFrame) => {
    readonly flushed: Promise<void>;
  } = () => ({
    flushed: Promise.resolve(),
  }),
): {
  attachment: NativeProviderConnectionAttachment;
  sent: NativeProviderHostFrame[];
  isClosed(): boolean;
} {
  const sent: NativeProviderHostFrame[] = [];
  let closed = false;
  const attachment = coordinator.attachConnection(connectionId, {
    enqueue: (frame) => {
      sent.push(frame);
      return enqueue(frame);
    },
    close: () => {
      closed = true;
    },
  });
  return { attachment, sent, isClosed: () => closed };
}

async function register(
  coordinator: HostNativeProviderCoordinator,
  connectionId: string,
): Promise<string> {
  const outcome = await coordinator.handlers['native.provider.register'](
    { capabilities: ['computer_use'] },
    { ...CONTEXT, connectionId },
  );
  if (!outcome.ok) assert.fail(outcome.error.message);
  return outcome.result.registrationId;
}

function acquire(
  coordinator: HostNativeProviderCoordinator,
  operationId: string,
  affinity?: string,
): HostNativeProviderInvocation {
  const acquisition = coordinator.acquireInvocation({
    operationId,
    sessionId: 'session',
    capability: 'computer_use',
    ...(affinity === undefined ? {} : { affinity }),
  });
  if (!acquisition.ok) assert.fail(acquisition.message);
  return acquisition.invocation;
}

function preflight(toolCallId: string) {
  return {
    kind: 'preflight' as const,
    context: { sessionId: 'session', turnId: 'turn', toolCallId },
  };
}

function capture(toolCallId: string) {
  return {
    kind: 'captureObservation' as const,
    input: { windowId: 1, includeScreenshot: true as const },
    context: { sessionId: 'session', turnId: 'turn', toolCallId },
  };
}

function preflightResult(): Extract<NativeProviderResultPayload, { kind: 'preflight' }> {
  return { kind: 'preflight', accessibility: true, screenRecording: true };
}

function captureResult(
  bytes: Buffer,
  sha256: string,
): Extract<NativeProviderResultPayload, { kind: 'captureObservation' }> {
  return {
    kind: 'captureObservation',
    observation: {
      observationId: 'observation',
      appId: 'app',
      pid: 1,
      windowId: 1,
      elements: [],
      screenshot: {
        image: {
          attachmentId: 'image',
          byteLength: bytes.byteLength,
          sha256,
          mimeType: 'image/png',
        },
        widthPx: 100,
        heightPx: 80,
      },
    },
  };
}

function requireSubcall(frame: NativeProviderHostFrame | undefined): NativeProviderSubcallFrame {
  assert.ok(frame && frame.kind === 'native.provider.subcall');
  return frame;
}

function requireSessionRelease(
  frame: NativeProviderHostFrame | undefined,
): NativeProviderSessionReleaseFrame {
  assert.ok(frame && frame.kind === 'native.provider.session_release');
  return frame;
}

function success(
  call: NativeProviderSubcallFrame,
  result: NativeProviderResultPayload,
): NativeProviderClientFrame {
  return {
    kind: 'native.provider.result',
    ...identity(call),
    ok: true,
    result,
  };
}

function failure(
  call: NativeProviderSubcallFrame,
  code: 'operation_failed' | 'outcome_unknown',
): NativeProviderClientFrame {
  return {
    kind: 'native.provider.result',
    ...identity(call),
    ok: false,
    error: { code },
  };
}

function chunk(
  call: NativeProviderSubcallFrame,
  attachmentId: string,
  index: number,
  bytes: Buffer,
): NativeProviderClientFrame {
  return {
    kind: 'native.provider.chunk',
    ...identity(call),
    attachmentId,
    index,
    data: bytes.toString('base64'),
  };
}

function identity(call: NativeProviderSubcallFrame) {
  return {
    hostEpoch: call.hostEpoch,
    operationId: call.operationId,
    subcallId: call.subcallId,
    ordinal: call.ordinal,
    bindingId: call.bindingId,
  };
}

function digest(bytes: Buffer): string {
  return createHash('sha256').update(bytes).digest('hex');
}

function signal(): AbortSignal {
  return new AbortController().signal;
}

function turn(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}
