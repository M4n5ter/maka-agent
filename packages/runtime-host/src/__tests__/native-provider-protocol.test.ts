import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import {
  decodeClientFrame,
  decodeHostFrame,
  HOST_OPERATION_SPECS,
  NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES,
  NATIVE_PROVIDER_MAX_APPS,
  NATIVE_PROVIDER_MAX_ATTACHMENT_BYTES,
  NATIVE_PROVIDER_MAX_ATTACHMENTS_PER_RESULT,
  NATIVE_PROVIDER_MAX_DISPLAYS,
  NATIVE_PROVIDER_MAX_ELEMENTS,
  NATIVE_PROVIDER_MAX_INLINE_PAYLOAD_BYTES,
  NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS,
  NATIVE_PROVIDER_MAX_RESULT_ATTACHMENT_BYTES,
  NATIVE_PROVIDER_MAX_SUBCALLS_PER_INVOCATION,
  NATIVE_PROVIDER_MAX_WINDOWS,
  NATIVE_PROVIDER_MAX_WINDOWS_PER_APP,
  RuntimeHostProtocolError,
} from '../protocol/index.js';

const invocationIdentity = {
  hostEpoch: 'epoch-1',
  operationId: 'operation-1',
  bindingId: 'binding-1',
} as const;

const subcallIdentity = {
  ...invocationIdentity,
  subcallId: 'subcall-1',
  ordinal: 1,
} as const;

const context = {
  sessionId: 'session-1',
  turnId: 'turn-1',
  toolCallId: 'tool-call-1',
  backendObservationId: 'backend-observation-1',
  boundAction: {
    frameId: 'frame-1',
    epoch: 1,
    target: {
      pid: 42,
      windowId: 7,
      bundleId: 'dev.maka.app',
      appName: 'Maka',
      title: 'Workspace',
      bounds: { x: 10, y: 20, width: 800, height: 600 },
      sourceBoundsPx: { x: 0, y: 0, width: 1600, height: 1200 },
      zIndex: 2,
      contentFingerprint: 'b'.repeat(64),
    },
    display: {
      displayId: 'display-1',
      logicalBounds: { x: 0, y: 0, width: 1440, height: 900 },
      sourceBoundsPx: { x: 0, y: 0, width: 2880, height: 1800 },
      scaleFactor: 2,
    },
    elementId: 'element-1',
    sourceCoordinate: { x: 200, y: 100 },
    windowCoordinate: { x: 100, y: 50 },
    coordinateSpace: 'window-screenshot-local' as const,
  },
} as const;

const attachment = {
  attachmentId: 'attachment-1',
  byteLength: 128,
  sha256: 'a'.repeat(64),
  mimeType: 'image/png' as const,
};

const screenshot = {
  image: attachment,
  widthPx: 800,
  heightPx: 600,
};

const observation = {
  observationId: 'observation-1',
  appId: 'dev.maka.app',
  pid: 42,
  windowId: 7,
  windowTitle: 'Workspace',
  capturedAt: 123,
  windowBounds: { x: 10, y: 20, width: 800, height: 600 },
  sourceBoundsPx: { x: 0, y: 0, width: 1600, height: 1200 },
  zIndex: 2,
  bundleId: 'dev.maka.app',
  contentFingerprint: 'b'.repeat(64),
  displays: [
    {
      displayId: 'display-1',
      logicalBounds: { x: 0, y: 0, width: 1440, height: 900 },
      sourceBoundsPx: { x: 0, y: 0, width: 2880, height: 1800 },
      scaleFactor: 2,
    },
  ],
  elements: [
    {
      elementId: 'element-1',
      role: 'button',
      label: 'Save',
      value: '',
      frame: { x: 20, y: 30, width: 80, height: 24 },
      identity: { role: 'button', label: 'Save', value: '' },
    },
  ],
} as const;

describe('Native Provider protocol', () => {
  test('keeps register and unregister as the only ordinary operations', () => {
    assert.deepEqual(
      Object.keys(HOST_OPERATION_SPECS).filter((key) => key.startsWith('native.provider.')),
      ['native.provider.register', 'native.provider.unregister'],
    );
    assert.deepEqual(metadata('native.provider.register'), ['control', 'none', 'ready']);
    assert.deepEqual(metadata('native.provider.unregister'), ['control', 'none', 'ready']);

    const register = HOST_OPERATION_SPECS['native.provider.register'];
    const unregister = HOST_OPERATION_SPECS['native.provider.unregister'];
    assert.deepEqual(register.decodeInput({ capabilities: ['computer_use'] }), {
      capabilities: ['computer_use'],
    });
    assert.deepEqual(register.decodeOutput({ registrationId: 'registration-1' }), {
      registrationId: 'registration-1',
    });
    assert.deepEqual(unregister.decodeInput({ registrationId: 'registration-1' }), {
      registrationId: 'registration-1',
    });
    assert.deepEqual(unregister.decodeOutput({ registrationId: 'registration-1' }), {
      registrationId: 'registration-1',
    });
    assert.throws(
      () => register.decodeInput({ capabilities: ['computer_use'], legacy: true }),
      isInvalidFrame,
    );
  });

  test('decodes all six closed typed subcalls under one invocation identity', () => {
    const subcalls = [
      { kind: 'preflight', context },
      { kind: 'listApps', context },
      {
        kind: 'observeApp',
        input: { app: 'Maka', includeScreenshot: false },
        context,
      },
      {
        kind: 'runSemantic',
        action: {
          type: 'set_value',
          observationId: 'observation-1',
          elementId: 'element-1',
          value: 'done',
          elementIdentity: { role: 'text_field', label: 'Status', value: '' },
        },
        context,
      },
      {
        kind: 'captureObservation',
        input: { windowId: 7, includeScreenshot: true },
        context,
      },
      {
        kind: 'run',
        action: {
          type: 'scroll',
          coordinate: { x: 100, y: 200 },
          scrollDirection: 'down',
          scrollAmount: 3,
          text: 'results',
        },
        context,
      },
    ] as const;

    for (const [index, subcall] of subcalls.entries()) {
      const frame = subcallFrame(subcall, index + 1);
      assert.deepEqual(decodeHostFrame(frame), frame);
      const withoutContext = { ...subcall } as Record<string, unknown>;
      delete withoutContext.context;
      assert.throws(() => decodeHostFrame(subcallFrame(withoutContext, index + 1)), isInvalidFrame);
    }
  });

  test('requires invocation context and keeps backend observation affinity opaque', () => {
    const contextWithoutAffinity = {
      sessionId: context.sessionId,
      turnId: context.turnId,
      toolCallId: context.toolCallId,
    };
    const preflight = subcallFrame({
      kind: 'preflight',
      context: contextWithoutAffinity,
    });
    assert.deepEqual(decodeHostFrame(preflight), preflight);

    for (const sensitive of [
      { page: { pageTargetId: 'page-1' } },
      { cdpPort: 9222 },
      { token: 'native-token' },
      { path: '/private/observation' },
      { rawResponse: {} },
      { secret: 'secret' },
    ]) {
      assert.throws(
        () =>
          decodeHostFrame(
            subcallFrame({
              kind: 'listApps',
              context: { ...context, ...sensitive },
            }),
          ),
        isInvalidFrame,
      );
    }
    assert.throws(
      () =>
        decodeHostFrame(
          subcallFrame({
            kind: 'preflight',
            context: { ...context, backendObservationId: 'x'.repeat(129) },
          }),
        ),
      isInvalidFrame,
    );
  });

  test('requires an app or windowId for observation subcalls', () => {
    for (const subcall of [
      {
        kind: 'observeApp',
        input: { includeScreenshot: false },
        context,
      },
      {
        kind: 'captureObservation',
        input: { includeScreenshot: true },
        context,
      },
      {
        kind: 'captureObservation',
        input: { app: 'Maka', includeScreenshot: false },
        context,
      },
    ]) {
      assert.throws(() => decodeHostFrame(subcallFrame(subcall)), isInvalidFrame);
    }
  });

  test('requires every transient and durable identity field on subcalls, results, and chunks', () => {
    const frames = [
      subcallFrame({ kind: 'preflight', context }),
      resultFrame({
        kind: 'preflight',
        accessibility: true,
        screenRecording: true,
      }),
      {
        kind: 'native.provider.chunk',
        ...subcallIdentity,
        attachmentId: 'attachment-1',
        index: 0,
        data: Buffer.from('image').toString('base64'),
      },
    ];
    for (const frame of frames) {
      for (const field of ['hostEpoch', 'operationId', 'subcallId', 'ordinal', 'bindingId']) {
        const missing = { ...frame } as Record<string, unknown>;
        delete missing[field];
        assert.throws(
          () =>
            frame.kind === 'native.provider.subcall'
              ? decodeHostFrame(missing)
              : decodeClientFrame(missing),
          isInvalidFrame,
        );
      }
    }

    const rebound = {
      ...resultFrame({
        kind: 'preflight',
        accessibility: true,
        screenRecording: false,
      }),
      hostEpoch: 'epoch-2',
      operationId: 'operation-2',
      subcallId: 'subcall-2',
      ordinal: 2,
      bindingId: 'binding-2',
    } as const;
    assert.deepEqual(decodeClientFrame(rebound), rebound);
    assert.notDeepEqual(
      identityOf(decodeClientFrame(rebound)),
      identityOf(
        decodeClientFrame(
          resultFrame({
            kind: 'preflight',
            accessibility: true,
            screenRecording: false,
          }),
        ),
      ),
    );

    for (const ordinal of [0, NATIVE_PROVIDER_MAX_SUBCALLS_PER_INVOCATION + 1]) {
      assert.throws(
        () =>
          decodeHostFrame({
            ...subcallFrame({ kind: 'preflight', context }),
            ordinal,
          }),
        isInvalidFrame,
      );
    }
  });

  test('decodes one-way invocation cancel and release controls', () => {
    const release = {
      kind: 'native.provider.release' as const,
      ...invocationIdentity,
    };
    assert.deepEqual(decodeHostFrame(release), release);
    assert.deepEqual(decodeHostFrame({ kind: 'native.provider.cancel', ...subcallIdentity }), {
      kind: 'native.provider.cancel',
      ...subcallIdentity,
    });

    for (const invalid of [{ ...release, subcallId: 'not-allowed' }]) {
      assert.throws(() => decodeHostFrame(invalid), isInvalidFrame);
    }
  });

  test('decodes acknowledged attachment-scoped Session release with exact identity', () => {
    const identity = {
      hostEpoch: 'epoch-1',
      registrationId: 'registration-1',
      releaseId: 'release-1',
      sessionId: 'session-1',
    } as const;
    const release = {
      kind: 'native.provider.session_release' as const,
      ...identity,
    };
    const acknowledged = {
      kind: 'native.provider.session_released' as const,
      ...identity,
    };
    assert.deepEqual(decodeHostFrame(release), release);
    assert.deepEqual(decodeClientFrame(acknowledged), acknowledged);
    for (const field of ['hostEpoch', 'registrationId', 'releaseId', 'sessionId']) {
      const invalidRelease = { ...release } as Record<string, unknown>;
      const invalidAcknowledgement = { ...acknowledged } as Record<string, unknown>;
      delete invalidRelease[field];
      delete invalidAcknowledgement[field];
      assert.throws(() => decodeHostFrame(invalidRelease), isInvalidFrame);
      assert.throws(() => decodeClientFrame(invalidAcknowledgement), isInvalidFrame);
    }
  });

  test('decodes matching closed results for all six subcall kinds', () => {
    const runResult = {
      outcome: {
        ok: true as const,
        tier: 'ax' as const,
        verified: true,
        effect: 'confirmed' as const,
        completedSubSteps: 1,
      },
      resolvedScreenPoint: { x: 100, y: 200 },
      observation,
    };
    const payloads = [
      { kind: 'preflight', accessibility: true, screenRecording: false },
      {
        kind: 'listApps',
        apps: [
          {
            appId: 'dev.maka.app',
            pid: 42,
            name: 'Maka',
            windowCount: 1,
            windows: [{ windowId: 7, title: 'Workspace' }],
          },
        ],
      },
      { kind: 'observeApp', observation },
      { kind: 'runSemantic', result: runResult },
      {
        kind: 'captureObservation',
        observation: { ...observation, screenshot },
      },
      {
        kind: 'run',
        result: {
          outcome: runResult.outcome,
          resolvedScreenPoint: runResult.resolvedScreenPoint,
          screenshot,
        },
      },
    ] as const;
    for (const payload of payloads) {
      const frame = resultFrame(payload);
      assert.deepEqual(decodeClientFrame(frame), frame);
    }

    const failure = {
      kind: 'native.provider.result' as const,
      ...subcallIdentity,
      ok: false as const,
      error: { code: 'outcome_unknown' as const },
    };
    assert.deepEqual(decodeClientFrame(failure), failure);
    assert.throws(
      () =>
        decodeClientFrame({
          ...failure,
          error: { ...failure.error, message: 'raw failure' },
        }),
      isInvalidFrame,
    );
  });

  test('rejects page identity, element tokens, raw responses, paths, secrets, and evidence text', () => {
    const sensitiveObservationFields = [
      { page: { cdpPort: 9222 } },
      { cdpPort: 9222 },
      { pageTargetId: 'page-1' },
      { pageUrl: 'https://private.example' },
      { targetUrlContains: 'private.example' },
      { documentFingerprint: 'fingerprint' },
      { path: '/private/capture.png' },
      { rawResponse: { native: true } },
      { secret: 'secret' },
    ];
    for (const sensitive of sensitiveObservationFields) {
      assert.throws(
        () =>
          decodeClientFrame(
            resultFrame({
              kind: 'observeApp',
              observation: { ...observation, ...sensitive },
            }),
          ),
        isInvalidFrame,
      );
    }

    for (const identity of [
      { role: 'button', token: 'native-token' },
      { role: 'button', endpoint: 'ws://localhost/devtools' },
    ]) {
      assert.throws(
        () =>
          decodeClientFrame(
            resultFrame({
              kind: 'observeApp',
              observation: {
                ...observation,
                elements: [{ elementId: 'element-1', role: 'button', identity }],
              },
            }),
          ),
        isInvalidFrame,
      );
    }

    for (const outcome of [
      { ok: false, error: 'capture_failed', message: 'raw native message' },
      { ok: true, tier: 'ax', evidence: { reason: 'raw reason' } },
      { ok: true, tier: 'ax', evidence: { path: '/private' } },
      { ok: true, tier: 'ax', rawResponse: {} },
    ]) {
      assert.throws(
        () => decodeClientFrame(resultFrame({ kind: 'run', result: { outcome } })),
        isInvalidFrame,
      );
    }

    assert.throws(
      () =>
        decodeHostFrame(
          subcallFrame({
            kind: 'runSemantic',
            action: {
              type: 'click_element',
              observationId: 'observation-1',
              elementId: 'element-1',
              elementIdentity: { role: 'button', token: 'native-token' },
            },
            context,
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeHostFrame(
          subcallFrame({
            kind: 'run',
            action: { type: 'wait', durationMs: 1 },
            context: {
              ...context,
              boundAction: {
                ...context.boundAction,
                target: {
                  ...context.boundAction.target,
                  page: { cdpPort: 9222 },
                },
              },
            },
          }),
        ),
      isInvalidFrame,
    );
  });

  test('enforces array, string, number, and inline payload bounds without truncation', () => {
    assert.equal(NATIVE_PROVIDER_MAX_APPS, 128);
    assert.equal(NATIVE_PROVIDER_MAX_WINDOWS_PER_APP, 64);
    assert.equal(NATIVE_PROVIDER_MAX_WINDOWS, 512);
    assert.equal(NATIVE_PROVIDER_MAX_DISPLAYS, 16);
    assert.equal(NATIVE_PROVIDER_MAX_ELEMENTS, 500);
    assert.equal(NATIVE_PROVIDER_MAX_INLINE_PAYLOAD_BYTES, 60 * 1024);

    const app = { appId: 'app', pid: 1, windowCount: 0 };
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'listApps',
            apps: Array.from({ length: NATIVE_PROVIDER_MAX_APPS + 1 }, () => app),
          }),
        ),
      isInvalidFrame,
    );
    const maximumDeclaredWindows = resultFrame({
      kind: 'listApps',
      apps: [
        {
          appId: 'app',
          pid: 1,
          windowCount: NATIVE_PROVIDER_MAX_WINDOWS,
          windows: Array.from({ length: NATIVE_PROVIDER_MAX_WINDOWS_PER_APP }, (_, index) => ({
            windowId: index + 1,
          })),
        },
      ],
    });
    assert.deepEqual(decodeClientFrame(maximumDeclaredWindows), maximumDeclaredWindows);
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'listApps',
            apps: [
              {
                appId: 'app',
                pid: 1,
                windowCount: NATIVE_PROVIDER_MAX_WINDOWS,
                windows: Array.from(
                  { length: NATIVE_PROVIDER_MAX_WINDOWS_PER_APP + 1 },
                  (_, index) => ({ windowId: index + 1 }),
                ),
              },
            ],
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'listApps',
            apps: [
              { appId: 'app-1', pid: 1, windowCount: 256 },
              { appId: 'app-2', pid: 2, windowCount: 257 },
            ],
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'listApps',
            apps: [
              {
                appId: 'app',
                pid: 1,
                windowCount: NATIVE_PROVIDER_MAX_WINDOWS + 1,
              },
            ],
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'observeApp',
            observation: {
              ...observation,
              displays: Array.from(
                { length: NATIVE_PROVIDER_MAX_DISPLAYS + 1 },
                () => observation.displays[0],
              ),
            },
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'observeApp',
            observation: {
              ...observation,
              elements: Array.from({ length: NATIVE_PROVIDER_MAX_ELEMENTS + 1 }, (_, index) => ({
                elementId: `element-${index}`,
                role: 'button',
              })),
            },
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'observeApp',
            observation: {
              ...observation,
              elements: Array.from({ length: NATIVE_PROVIDER_MAX_ELEMENTS }, (_, index) => ({
                elementId: `element-${index}`,
                role: 'button',
                value: 'x'.repeat(200),
              })),
            },
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeHostFrame(
          subcallFrame({
            kind: 'run',
            action: { type: 'wait', durationMs: Infinity },
            context,
          }),
        ),
      isInvalidFrame,
    );
    assert.throws(
      () =>
        decodeHostFrame(
          subcallFrame({
            kind: 'run',
            action: { type: 'type', text: 'x'.repeat(8_001) },
            context,
          }),
        ),
      isInvalidFrame,
    );
  });

  test('binds chunks and the single screenshot attachment to the full subcall identity', () => {
    assert.equal(NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES, 32 * 1024);
    assert.equal(NATIVE_PROVIDER_MAX_ATTACHMENTS_PER_RESULT, 1);
    assert.equal(NATIVE_PROVIDER_MAX_ATTACHMENT_BYTES, 8 * 1024 * 1024);
    assert.equal(NATIVE_PROVIDER_MAX_RESULT_ATTACHMENT_BYTES, 8 * 1024 * 1024);
    assert.equal(NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS, 8);

    const chunkFrame = (data: string) => ({
      kind: 'native.provider.chunk' as const,
      ...subcallIdentity,
      attachmentId: attachment.attachmentId,
      index: 0,
      data,
    });
    const maximum = Buffer.alloc(NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES, 0xa5).toString('base64');
    const oversized = Buffer.alloc(NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES + 1, 0xa5).toString(
      'base64',
    );
    assert.deepEqual(decodeClientFrame(chunkFrame(maximum)), chunkFrame(maximum));
    assert.throws(() => decodeClientFrame(chunkFrame(oversized)), isInvalidFrame);

    const capture = resultFrame({
      kind: 'captureObservation',
      observation: { ...observation, screenshot },
    });
    assert.deepEqual(decodeClientFrame(capture), capture);
    assert.throws(
      () =>
        decodeClientFrame({
          ...capture,
          result: {
            ...capture.result,
            observation: {
              ...capture.result.observation,
              screenshot: {
                ...screenshot,
                image: {
                  ...attachment,
                  byteLength: NATIVE_PROVIDER_MAX_ATTACHMENT_BYTES + 1,
                },
              },
            },
          },
        }),
      isInvalidFrame,
    );

    assert.throws(
      () =>
        decodeClientFrame(
          resultFrame({
            kind: 'run',
            result: {
              outcome: { ok: true, tier: 'ax' },
              screenshot,
              observation: { ...observation, screenshot },
            },
          }),
        ),
      isInvalidFrame,
    );
  });
});

function subcallFrame<T>(
  subcall: T,
  ordinal = 1,
): {
  kind: 'native.provider.subcall';
  hostEpoch: string;
  operationId: string;
  subcallId: string;
  ordinal: number;
  bindingId: string;
  capability: 'computer_use';
  subcall: T;
} {
  return {
    kind: 'native.provider.subcall',
    ...subcallIdentity,
    ordinal,
    capability: 'computer_use',
    subcall,
  };
}

function resultFrame<T>(result: T): {
  kind: 'native.provider.result';
  hostEpoch: string;
  operationId: string;
  subcallId: string;
  ordinal: number;
  bindingId: string;
  ok: true;
  result: T;
} {
  return {
    kind: 'native.provider.result',
    ...subcallIdentity,
    ok: true,
    result,
  };
}

function identityOf(frame: unknown): unknown {
  if (!frame || typeof frame !== 'object') assert.fail('expected a frame');
  const record = frame as Record<string, unknown>;
  return {
    hostEpoch: record.hostEpoch,
    operationId: record.operationId,
    subcallId: record.subcallId,
    ordinal: record.ordinal,
    bindingId: record.bindingId,
  };
}

function metadata(
  operation: 'native.provider.register' | 'native.provider.unregister',
): readonly string[] {
  const { mode, retry, admission } = HOST_OPERATION_SPECS[operation];
  return [mode, retry, admission];
}

function isInvalidFrame(error: unknown): error is RuntimeHostProtocolError {
  return error instanceof RuntimeHostProtocolError && error.code === 'invalid_frame';
}
