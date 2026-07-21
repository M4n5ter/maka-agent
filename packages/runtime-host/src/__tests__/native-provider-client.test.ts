import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer, type Server } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  connectRuntimeHost,
  NativeCapabilityProvider,
  type RuntimeHostConnection,
} from '../client/index.js';
import type { ClientNativeProviderAttachment } from '../client/native-provider.js';
import { prepareRuntimeHostEndpoint } from '../control/endpoint.js';
import { removeHostRegistration, writeHostRegistration } from '../control/registration.js';
import {
  decodeClientFrame,
  decodeNativeProviderClientFrame,
  encodeProtocolFrame,
  RUNTIME_HOST_PROTOCOL_VERSION,
  RUNTIME_HOST_REGISTRATION_SCHEMA_VERSION,
  type HostFrame,
  type NativeProviderClientFrame,
  type NativeProviderSubcall,
  type NativeProviderSubcallFrame,
  type RequestFrame,
} from '../protocol/index.js';
import { FramedTransport } from '../transport/framed-transport.js';
import {
  prepareStorageRootControlDirectory,
  resolveStorageRoot,
} from '@maka/storage/root-authority';

const PROTOCOL = {
  min: RUNTIME_HOST_PROTOCOL_VERSION,
  max: RUNTIME_HOST_PROTOCOL_VERSION,
} as const;

test('pumps ordinary responses while a subcall blocks and waits for real settlement after cancel', async () => {
  const entered = deferred<void>();
  const aborted = deferred<void>();
  const settle = deferred<void>();
  const resultObserved = deferred<void>();
  await withProtocolPeer(
    async (transport, hostEpoch) => {
      const register = await acceptConnectionAndReadRegister(transport, hostEpoch);
      const subcall = subcallFrame(hostEpoch, 'operation-blocked', 1, {
        kind: 'preflight',
        context: context(),
      });
      await transport.writeEncoded(
        Buffer.concat([
          encodeProtocolFrame(registerSuccess(register, 'registration-blocked')),
          encodeProtocolFrame(subcall),
        ]),
      );
      await entered.promise;

      await answerStatus(transport, hostEpoch);
      await transport.write({
        kind: 'native.provider.cancel',
        hostEpoch,
        operationId: subcall.operationId,
        subcallId: subcall.subcallId,
        ordinal: subcall.ordinal,
        bindingId: subcall.bindingId,
      });
      await aborted.promise;

      // A second ordinary response must overtake the still-unsettled backend handler.
      await answerStatus(transport, hostEpoch);
      settle.resolve();
      const result = await readNativeFrame(transport, 'native.provider.result');
      assert.equal(result.operationId, subcall.operationId);
      assert.equal(result.ok, false);
      await transport.write(releaseFrame(hostEpoch, subcall.operationId, subcall.bindingId));
      resultObserved.resolve();

      const unregister = await readRequest(transport, 'native.provider.unregister');
      await transport.write(unregisterSuccess(unregister, 'registration-blocked'));
    },
    async (connection) => {
      const provider = new NativeCapabilityProvider({
        capabilities: ['computer_use'],
        releaseSession: () => {},
        handle: async (_frame, { signal }) => {
          entered.resolve();
          await waitForAbort(signal);
          aborted.resolve();
          await settle.promise;
          return { ok: false, code: 'operation_failed' };
        },
      });
      const registration = await connection.registerNativeProvider(provider);
      assert.equal((await connection.status(5_000)).hostEpoch, connection.hostEpoch);
      assert.equal((await connection.status(5_000)).hostEpoch, connection.hostEpoch);
      await resultObserved.promise;
      await registration.unregister();
    },
  );
});

test('enforces invocation identity and ordinary release does not clear its Session', async () => {
  const frames: NativeProviderClientFrame[] = [];
  const releases: string[] = [];
  const provider = new NativeCapabilityProvider({
    capabilities: ['computer_use'],
    releaseSession: (sessionId) => {
      releases.push(sessionId);
    },
    handle: async () => ({
      ok: true,
      complete: () => ({
        kind: 'preflight',
        accessibility: true,
        screenRecording: true,
      }),
    }),
  });
  const attachment = await directAttachment(provider, 'epoch-sequence', frames);
  const first = subcallFrame('epoch-sequence', 'operation-sequence', 1, {
    kind: 'preflight',
    context: context(),
  });
  attachment.acceptSubcall(first);
  await waitForResult(frames, first.subcallId);

  const second = subcallFrame('epoch-sequence', first.operationId, 2, {
    kind: 'preflight',
    context: context(),
  });
  attachment.acceptSubcall(second);
  await waitForResult(frames, second.subcallId);
  assert.throws(
    () =>
      attachment.acceptSubcall({
        ...subcallFrame('epoch-sequence', first.operationId, 3, {
          kind: 'preflight',
          context: context(),
        }),
        bindingId: 'changed-binding',
      }),
    /changed.*identity/,
  );
  assert.throws(
    () =>
      attachment.acceptSubcall(
        subcallFrame('epoch-sequence', first.operationId, 4, {
          kind: 'preflight',
          context: context(),
        }),
      ),
    /non-contiguous/,
  );

  attachment.acceptRelease(releaseFrame('epoch-sequence', first.operationId, first.bindingId));
  assert.deepEqual(releases, []);

  // release leaves no tombstone: the durable operation identity may be admitted anew.
  const reused = {
    ...subcallFrame('epoch-sequence', first.operationId, 1, {
      kind: 'preflight',
      context: context(),
    }),
    subcallId: 'subcall-reused-operation-sequence',
  };
  attachment.acceptSubcall(reused);
  await waitForResult(frames, reused.subcallId);
  attachment.acceptRelease(releaseFrame('epoch-sequence', reused.operationId, reused.bindingId));
  attachment.acceptSessionRelease(
    sessionReleaseFrame('epoch-sequence', 'registration-epoch-sequence', 'release-session-1'),
  );
  await waitForSessionReleased(frames, 'release-session-1');
  assert.deepEqual(releases, ['session-1']);
  attachment.sealAdmission();
  await attachment.drained;
});

test('ack loss permits reattach after remaining cleanup, while callback failure blocks it', async () => {
  const entered = deferred<void>();
  const aborted = deferred<void>();
  const settle = deferred<void>();
  const cleanup = deferred<void>();
  const transportFailed = deferred<void>();
  const cleanupFailed = deferred<void>();
  const frames: NativeProviderClientFrame[] = [];
  const releasedSessions: string[] = [];
  const provider = new NativeCapabilityProvider({
    capabilities: ['computer_use'],
    releaseSession: async (sessionId) => {
      releasedSessions.push(sessionId);
      if (sessionId === 'session-3') throw new Error('permanent cleanup failure');
      await cleanup.promise;
    },
    handle: async (frame, { signal }) => {
      if (frame.subcall.context.sessionId !== 'session-1') {
        return { ok: false, code: 'operation_failed' };
      }
      entered.resolve();
      await waitForAbort(signal);
      aborted.resolve();
      await settle.promise;
      return { ok: false, code: 'operation_failed' };
    },
  });
  const oldAttachment = await provider.attach({
    hostEpoch: 'epoch-old',
    send: async (frame) => {
      frames.push(frame);
      if (frame.kind === 'native.provider.session_released') throw new Error('ack lost');
    },
    fail: () => transportFailed.resolve(),
  });
  oldAttachment.bindRegistration('registration-old');
  const subcall = subcallFrame('epoch-old', 'operation-old', 1, {
    kind: 'preflight',
    context: context(),
  });
  oldAttachment.acceptSubcall(subcall);
  await entered.promise;
  const secondSession = subcallFrame('epoch-old', 'operation-old-s2', 1, {
    kind: 'preflight',
    context: { ...context(), sessionId: 'session-2' },
  });
  oldAttachment.acceptSubcall(secondSession);
  await waitForResult(frames, secondSession.subcallId);
  oldAttachment.acceptRelease(
    releaseFrame(secondSession.hostEpoch, secondSession.operationId, secondSession.bindingId),
  );

  let attachedNew = false;
  const newFrames: NativeProviderClientFrame[] = [];
  const newAttachmentTask = provider
    .attach({
      hostEpoch: 'epoch-new',
      send: async (frame) => {
        newFrames.push(frame);
      },
      fail: () => cleanupFailed.resolve(),
    })
    .then((attachment) => {
      attachedNew = true;
      return attachment;
    });
  await immediate();
  assert.equal(attachedNew, false);
  oldAttachment.acceptCancel({
    kind: 'native.provider.cancel',
    hostEpoch: subcall.hostEpoch,
    operationId: subcall.operationId,
    subcallId: subcall.subcallId,
    ordinal: subcall.ordinal,
    bindingId: subcall.bindingId,
  });
  await aborted.promise;
  settle.resolve();
  await waitForResult(frames, subcall.subcallId);
  oldAttachment.acceptRelease(
    releaseFrame(subcall.hostEpoch, subcall.operationId, subcall.bindingId),
  );
  oldAttachment.acceptSessionRelease(
    sessionReleaseFrame('epoch-old', 'registration-old', 'release-old'),
  );
  await immediate();
  assert.deepEqual(releasedSessions, ['session-1']);
  assert.equal(attachedNew, false);
  cleanup.resolve();
  await transportFailed.promise;
  await oldAttachment.drained;
  const newAttachment = await newAttachmentTask;
  assert.equal(attachedNew, true);
  assert.deepEqual(releasedSessions, ['session-1', 'session-2']);

  newAttachment.bindRegistration('registration-new');
  const thirdSession = subcallFrame('epoch-new', 'operation-cleanup-failure', 1, {
    kind: 'preflight',
    context: { ...context(), sessionId: 'session-3' },
  });
  newAttachment.acceptSubcall(thirdSession);
  await waitForResult(newFrames, thirdSession.subcallId);
  newAttachment.acceptRelease(
    releaseFrame(thirdSession.hostEpoch, thirdSession.operationId, thirdSession.bindingId),
  );
  newAttachment.detach();
  await cleanupFailed.promise;
  await newAttachment.drained;
  assert.deepEqual(releasedSessions, ['session-1', 'session-2', 'session-3']);
  await assert.rejects(
    provider.attach({
      hostEpoch: 'epoch-after-cleanup-failure',
      send: async () => undefined,
      fail: (error) => assert.fail(error.message),
    }),
    /failed during session cleanup/,
  );
});

test('unregister is an ordinary request and rejected registration rolls back for retry', async () => {
  await withProtocolPeer(
    async (transport, hostEpoch) => {
      const rejected = await acceptConnectionAndReadRegister(transport, hostEpoch);
      await transport.write({
        requestId: rejected.requestId,
        operation: 'native.provider.register',
        ok: false,
        error: {
          code: 'operation_conflict',
          message: 'Provider registration rejected',
        },
      });
      const accepted = await readRequest(transport, 'native.provider.register');
      await transport.write(registerSuccess(accepted, 'registration-after-rollback'));
      const unregister = await readRequest(transport, 'native.provider.unregister');
      assert.deepEqual(unregister.input, {
        registrationId: 'registration-after-rollback',
      });
      await transport.write(unregisterSuccess(unregister, 'registration-after-rollback'));
    },
    async (connection) => {
      const provider = new NativeCapabilityProvider({
        capabilities: ['computer_use'],
        releaseSession: () => {},
        handle: async () => ({ ok: false, code: 'operation_failed' }),
      });
      await assert.rejects(
        () => connection.registerNativeProvider(provider),
        /Provider registration rejected/,
      );
      const registration = await connection.registerNativeProvider(provider);
      await registration.unregister();
      await registration.drained;
    },
  );
});

test('reserves a connection synchronously against concurrent Native Provider registration', async () => {
  const registerReceived = deferred<void>();
  const acceptRegistration = deferred<void>();
  await withProtocolPeer(
    async (transport, hostEpoch) => {
      const register = await acceptConnectionAndReadRegister(transport, hostEpoch);
      registerReceived.resolve();
      await acceptRegistration.promise;
      await transport.write(registerSuccess(register, 'registration-reserved'));
      const unregister = await readRequest(transport, 'native.provider.unregister');
      await transport.write(unregisterSuccess(unregister, 'registration-reserved'));
    },
    async (connection) => {
      const firstProvider = new NativeCapabilityProvider({
        capabilities: ['computer_use'],
        releaseSession: () => {},
        handle: async () => ({ ok: false, code: 'operation_failed' }),
      });
      const secondProvider = new NativeCapabilityProvider({
        capabilities: ['computer_use'],
        releaseSession: () => {},
        handle: async () => ({ ok: false, code: 'operation_failed' }),
      });

      const firstRegistration = connection.registerNativeProvider(firstProvider);
      await assert.rejects(
        connection.registerNativeProvider(secondProvider),
        /already has a Native Provider registration/,
      );
      await registerReceived.promise;
      acceptRegistration.resolve();
      const registration = await firstRegistration;
      await registration.unregister();
    },
  );
});

async function directAttachment(
  provider: NativeCapabilityProvider,
  hostEpoch: string,
  frames: NativeProviderClientFrame[],
): Promise<ClientNativeProviderAttachment> {
  const attachment = await provider.attach({
    hostEpoch,
    send: async (frame) => {
      frames.push(decodeNativeProviderClientFrame(JSON.parse(JSON.stringify(frame))));
    },
    fail: (error) => assert.fail(error.message),
  });
  attachment.bindRegistration(`registration-${hostEpoch}`);
  return attachment;
}

function subcallFrame(
  hostEpoch: string,
  operationId: string,
  ordinal: number,
  subcall: NativeProviderSubcall,
): NativeProviderSubcallFrame {
  return {
    kind: 'native.provider.subcall',
    hostEpoch,
    operationId,
    subcallId: `subcall-${operationId}-${ordinal}`,
    ordinal,
    bindingId: `binding-${operationId}`,
    capability: 'computer_use',
    subcall,
  };
}

function context() {
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    toolCallId: 'tool-call-1',
  };
}

function releaseFrame(hostEpoch: string, operationId: string, bindingId: string) {
  return {
    kind: 'native.provider.release' as const,
    hostEpoch,
    operationId,
    bindingId,
  };
}

function sessionReleaseFrame(hostEpoch: string, registrationId: string, releaseId: string) {
  return {
    kind: 'native.provider.session_release' as const,
    hostEpoch,
    registrationId,
    releaseId,
    sessionId: 'session-1',
  };
}

async function waitForSessionReleased(frames: NativeProviderClientFrame[], releaseId: string) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const ack = frames.find(
      (frame) => frame.kind === 'native.provider.session_released' && frame.releaseId === releaseId,
    );
    if (ack) return ack;
    await immediate();
  }
  throw new Error(`Timed out waiting for Session release ${releaseId}`);
}

async function waitForResult(frames: NativeProviderClientFrame[], subcallId: string) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const result = frames.find(
      (frame) => frame.kind === 'native.provider.result' && frame.subcallId === subcallId,
    );
    if (result?.kind === 'native.provider.result') return result;
    await immediate();
  }
  throw new Error(`Timed out waiting for ${subcallId}`);
}

async function withProtocolPeer(
  serve: (transport: FramedTransport, hostEpoch: string) => Promise<void>,
  run: (connection: RuntimeHostConnection) => Promise<void>,
): Promise<void> {
  const base = await mkdtemp(join(tmpdir(), 'maka-runtime-host-native-provider-'));
  const capability = await resolveStorageRoot({
    path: join(base, 'root'),
    kind: 'interactive',
  });
  const { controlDirectory } = await prepareStorageRootControlDirectory(capability);
  const hostEpoch = randomUUID();
  const endpoint = await prepareRuntimeHostEndpoint({
    rootId: capability.rootId,
    hostEpoch,
  });
  const serverTask = deferred<void>();
  const server = createServer((socket) => {
    void serve(new FramedTransport(socket), hostEpoch).then(serverTask.resolve, serverTask.reject);
  });
  try {
    await listen(server, endpoint.path);
    await endpoint.prepareAfterListen();
    await writeHostRegistration(controlDirectory, {
      kind: 'maka-runtime-host',
      schemaVersion: RUNTIME_HOST_REGISTRATION_SCHEMA_VERSION,
      rootId: capability.rootId,
      hostEpoch,
      endpoint: endpoint.path,
      protocolMin: RUNTIME_HOST_PROTOCOL_VERSION,
      protocolMax: RUNTIME_HOST_PROTOCOL_VERSION,
      state: 'ready',
      pid: process.pid,
      createdAt: new Date().toISOString(),
    });
    const connected = await connectRuntimeHost({
      rootPath: join(base, 'root'),
      surface: 'tui',
      protocol: PROTOCOL,
    });
    assert.equal(connected.kind, 'connected');
    if (connected.kind !== 'connected') return;
    try {
      await run(connected.connection);
    } finally {
      await connected.connection.close();
    }
    await serverTask.promise;
  } finally {
    await closeServer(server);
    await removeHostRegistration(controlDirectory, hostEpoch).catch(() => undefined);
    await endpoint.cleanup().catch(() => undefined);
    await rm(base, { recursive: true, force: true });
  }
}

async function acceptConnectionAndReadRegister(transport: FramedTransport, hostEpoch: string) {
  const hello = decodeClientFrame(await transport.read(1_000));
  assert.ok('kind' in hello && hello.kind === 'hello');
  await transport.write({
    kind: 'accepted',
    hostEpoch,
    connectionId: 'connection-native-provider',
    selectedProtocol: RUNTIME_HOST_PROTOCOL_VERSION,
    state: 'ready',
  });
  return readRequest(transport, 'native.provider.register');
}

async function answerStatus(transport: FramedTransport, hostEpoch: string): Promise<void> {
  const status = await readRequest(transport, 'host.status');
  await transport.write({
    requestId: status.requestId,
    operation: 'host.status',
    ok: true,
    result: {
      hostEpoch,
      state: 'ready',
      connections: 1,
      activeOperations: 1,
      activeResidencies: 1,
    },
  });
}

async function readRequest<K extends RequestFrame['operation']>(
  transport: FramedTransport,
  operation: K,
) {
  const frame = decodeClientFrame(await transport.read(1_000));
  assert.ok(!('kind' in frame));
  assert.equal(frame.operation, operation);
  return frame as Extract<RequestFrame, { operation: K }>;
}

async function readNativeFrame<K extends NativeProviderClientFrame['kind']>(
  transport: FramedTransport,
  kind: K,
) {
  const frame = decodeClientFrame(await transport.read(1_000));
  assert.ok('kind' in frame && frame.kind === kind);
  return frame as Extract<NativeProviderClientFrame, { kind: K }>;
}

function registerSuccess(
  request: Extract<RequestFrame, { operation: 'native.provider.register' }>,
  registrationId: string,
): HostFrame {
  return {
    requestId: request.requestId,
    operation: 'native.provider.register',
    ok: true,
    result: { registrationId },
  };
}

function unregisterSuccess(
  request: Extract<RequestFrame, { operation: 'native.provider.unregister' }>,
  registrationId: string,
): HostFrame {
  return {
    requestId: request.requestId,
    operation: 'native.provider.unregister',
    ok: true,
    result: { registrationId },
  };
}

function waitForAbort(signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.resolve();
  return new Promise((resolve) =>
    signal.addEventListener('abort', () => resolve(), { once: true }),
  );
}

function immediate(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

function listen(server: Server, path: string): Promise<void> {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(path, resolve);
  });
}

function closeServer(server: Server): Promise<void> {
  if (!server.listening) return Promise.resolve();
  return new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
