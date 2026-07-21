import { createHash } from 'node:crypto';
import {
  decodeNativeProviderClientFrame,
  NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES,
  NATIVE_PROVIDER_MAX_ATTACHMENT_BYTES,
  NATIVE_PROVIDER_MAX_CAPABILITIES,
  NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS,
  NATIVE_PROVIDER_MAX_SUBCALLS_PER_INVOCATION,
  nativeProviderResultAttachmentRefs,
  type NativeProviderAttachmentRef,
  type NativeProviderCapability,
  type NativeProviderChunkFrame,
  type NativeProviderFailureCode,
  type NativeProviderReleaseFrame,
  type NativeProviderResultFrame,
  type NativeProviderResultPayload,
  type NativeProviderSessionReleaseFrame,
  type NativeProviderSessionReleasedFrame,
  type NativeProviderSubcallFrame,
  type NativeProviderCancelFrame,
} from '../protocol/index.js';

export const NATIVE_PROVIDER_DEFAULT_CHUNK_BYTES = NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES;

export type NativeCapability = NativeProviderCapability;

export interface NativeCapabilityAttachment {
  readonly attachmentId: string;
  readonly mimeType: 'image/png' | 'image/jpeg';
  readonly data: Uint8Array;
}

export type NativeCapabilityAttachmentRef = NativeProviderAttachmentRef;

export type NativeCapabilityHandlerOutcome =
  | {
      readonly ok: true;
      readonly attachment?: NativeCapabilityAttachment;
      readonly complete: (
        attachment?: NativeCapabilityAttachmentRef,
      ) => NativeProviderResultPayload;
    }
  | { readonly ok: false; readonly code: NativeProviderFailureCode };

export interface NativeCapabilityHandlerContext {
  readonly signal: AbortSignal;
}

export type NativeCapabilityHandler = (
  frame: NativeProviderSubcallFrame,
  context: NativeCapabilityHandlerContext,
) => Promise<NativeCapabilityHandlerOutcome>;

export interface NativeCapabilityProviderOptions {
  readonly capabilities: readonly NativeCapability[];
  readonly handle: NativeCapabilityHandler;
  readonly releaseSession: (sessionId: string) => void | Promise<void>;
  readonly chunkBytes?: number;
}

export interface NativeProviderRegistration {
  readonly registrationId: string;
  readonly drained: Promise<void>;
  unregister(timeoutMs?: number): Promise<void>;
}

export interface NativeProviderAttachmentTransport {
  readonly hostEpoch: string;
  send(
    frame:
      | NativeProviderChunkFrame
      | NativeProviderResultFrame
      | NativeProviderSessionReleasedFrame,
  ): Promise<void>;
  fail(error: Error): void;
}

interface Invocation {
  readonly operationId: string;
  readonly bindingId: string;
  readonly sessionId: string;
  readonly turnId: string;
  readonly toolCallId: string;
  nextOrdinal: number;
  active?: {
    readonly subcallId: string;
    readonly ordinal: number;
    readonly controller: AbortController;
    readonly settled: Promise<void>;
    readonly resolveSettled: () => void;
  };
}

interface SessionCleanup {
  readonly releaseId: string;
  readonly task: Promise<void>;
}

export class NativeCapabilityProvider {
  readonly capabilities: readonly NativeCapability[];
  readonly #options: NativeCapabilityProviderOptions;
  #current: ClientNativeProviderAttachment | undefined;
  #attachGate = Promise.resolve();

  constructor(options: NativeCapabilityProviderOptions) {
    if (options.capabilities.length === 0) {
      throw new RangeError('Native capability provider must offer at least one capability');
    }
    if (options.capabilities.length > NATIVE_PROVIDER_MAX_CAPABILITIES) {
      throw new RangeError('Native capability provider offers too many capabilities');
    }
    if (new Set(options.capabilities).size !== options.capabilities.length) {
      throw new RangeError('Native capability provider capabilities must be unique');
    }
    const chunkBytes = options.chunkBytes ?? NATIVE_PROVIDER_DEFAULT_CHUNK_BYTES;
    if (
      !Number.isSafeInteger(chunkBytes) ||
      chunkBytes < 1 ||
      chunkBytes > NATIVE_PROVIDER_ATTACHMENT_CHUNK_BYTES
    ) {
      throw new RangeError('Native capability provider chunkBytes must be between 1 and 32768');
    }
    this.capabilities = Object.freeze([...options.capabilities]);
    this.#options = { ...options, chunkBytes };
  }

  get drained(): Promise<void> {
    return this.#current?.drained ?? Promise.resolve();
  }

  attach(transport: NativeProviderAttachmentTransport): Promise<ClientNativeProviderAttachment> {
    const attached = this.#attachGate.then(async () => {
      if (this.#current) {
        await this.#current.drained;
        if (this.#current.failed) {
          throw new Error('Native capability provider attachment failed during session cleanup');
        }
      }
      const attachment = new ClientNativeProviderAttachment(
        transport,
        this.capabilities,
        this.#options,
      );
      this.#current = attachment;
      return attachment;
    });
    this.#attachGate = attached.then(
      () => undefined,
      () => undefined,
    );
    return attached;
  }
}

export class ClientNativeProviderAttachment {
  readonly capabilities: readonly NativeCapability[];
  readonly drained: Promise<void>;
  readonly #transport: NativeProviderAttachmentTransport;
  readonly #options: NativeCapabilityProviderOptions;
  readonly #invocations = new Map<string, Invocation>();
  readonly #seenSessions = new Set<string>();
  readonly #sessionCleanups = new Map<string, SessionCleanup>();
  readonly #chunkBytes: number;
  #registrationId: string | undefined;
  #admissionOpen = true;
  #sendResults = true;
  #transportFailed = false;
  #cleanupFailed = false;
  #drainStarted = false;
  #resolveDrained!: () => void;
  #writeTail = Promise.resolve();

  constructor(
    transport: NativeProviderAttachmentTransport,
    capabilities: readonly NativeCapability[],
    options: NativeCapabilityProviderOptions,
  ) {
    this.#transport = transport;
    this.capabilities = capabilities;
    this.#options = options;
    this.#chunkBytes = options.chunkBytes ?? NATIVE_PROVIDER_DEFAULT_CHUNK_BYTES;
    this.drained = new Promise((resolve) => {
      this.#resolveDrained = resolve;
    });
  }

  canAccept(capability: NativeCapability): boolean {
    return this.#admissionOpen && this.capabilities.includes(capability);
  }

  get failed(): boolean {
    return this.#cleanupFailed;
  }

  bindRegistration(registrationId: string): void {
    if (this.#registrationId) throw new Error('Native Provider attachment is already registered');
    this.#registrationId = registrationId;
  }

  hasInvocation(operationId: string): boolean {
    return this.#invocations.has(operationId);
  }

  acceptSubcall(frame: NativeProviderSubcallFrame): void {
    this.#requireEpoch(frame.hostEpoch);
    if (!this.#registrationId)
      throw new Error('Native Provider subcall arrived before registration');
    if (!this.#admissionOpen)
      throw new Error('Native Provider subcall arrived after admission closed');
    if (!this.capabilities.includes(frame.capability)) {
      throw new Error('Runtime Host called an unregistered Native Provider capability');
    }
    let invocation = this.#invocations.get(frame.operationId);
    const context = frame.subcall.context;
    if (this.#sessionCleanups.has(context.sessionId)) {
      throw new Error('Native Provider subcall arrived while its Session is releasing');
    }
    if (!invocation) {
      if (frame.ordinal !== 1)
        throw new Error('Native Provider invocation must begin at ordinal 1');
      if (this.#invocations.size >= NATIVE_PROVIDER_MAX_PENDING_INVOCATIONS) {
        throw new Error('Runtime Host exceeded the Native Provider pending invocation limit');
      }
      invocation = {
        operationId: frame.operationId,
        bindingId: frame.bindingId,
        sessionId: context.sessionId,
        turnId: context.turnId,
        toolCallId: context.toolCallId,
        nextOrdinal: 1,
      };
      this.#invocations.set(frame.operationId, invocation);
    }
    if (
      invocation.bindingId !== frame.bindingId ||
      invocation.sessionId !== context.sessionId ||
      invocation.turnId !== context.turnId ||
      invocation.toolCallId !== context.toolCallId
    ) {
      throw new Error('Runtime Host changed Native Provider invocation identity');
    }
    if (invocation.active)
      throw new Error('Runtime Host issued concurrent Native Provider subcalls');
    if (
      frame.ordinal !== invocation.nextOrdinal ||
      frame.ordinal > NATIVE_PROVIDER_MAX_SUBCALLS_PER_INVOCATION
    ) {
      throw new Error('Runtime Host issued a non-contiguous Native Provider subcall ordinal');
    }
    const active = {
      subcallId: frame.subcallId,
      ordinal: frame.ordinal,
      controller: new AbortController(),
      ...settlement(),
    };
    invocation.active = active;
    this.#seenSessions.add(context.sessionId);
    setImmediate(() => void this.#run(frame, invocation!, active));
  }

  acceptCancel(frame: NativeProviderCancelFrame): void {
    this.#requireEpoch(frame.hostEpoch);
    const invocation = this.#requireInvocation(frame.operationId, frame.bindingId);
    const active = invocation.active;
    if (!active || active.subcallId !== frame.subcallId || active.ordinal !== frame.ordinal) {
      throw new Error('Runtime Host cancelled a non-active Native Provider subcall');
    }
    active.controller.abort();
  }

  acceptRelease(frame: NativeProviderReleaseFrame): void {
    this.#requireEpoch(frame.hostEpoch);
    const invocation = this.#requireInvocation(frame.operationId, frame.bindingId);
    if (invocation.active)
      throw new Error('Runtime Host released an active Native Provider invocation');
    this.#invocations.delete(frame.operationId);
    this.#resolveDrainIfReady();
  }

  acceptSessionRelease(frame: NativeProviderSessionReleaseFrame): void {
    this.#requireEpoch(frame.hostEpoch);
    if (frame.registrationId !== this.#registrationId) {
      throw new Error('Native Provider Session release referenced a different registration');
    }
    if (!this.#sendResults) throw new Error('Native Provider Session release arrived after detach');
    const existing = this.#sessionCleanups.get(frame.sessionId);
    if (existing) {
      if (existing.releaseId !== frame.releaseId) {
        throw new Error('Native Provider Session release identity changed during cleanup');
      }
      return;
    }
    if (!this.#seenSessions.has(frame.sessionId)) {
      throw new Error('Native Provider Session release referenced an unseen Session');
    }
    const task = this.#releaseSession(frame);
    this.#sessionCleanups.set(frame.sessionId, {
      releaseId: frame.releaseId,
      task,
    });
    void task.catch((error: unknown) => this.#failTransport(asError(error)));
  }

  sealAdmission(): void {
    if (!this.#admissionOpen) return;
    this.#admissionOpen = false;
    this.#resolveDrainIfReady();
  }

  detach(): void {
    this.sealAdmission();
    if (!this.#sendResults) return;
    this.#sendResults = false;
    for (const invocation of this.#invocations.values()) invocation.active?.controller.abort();
    this.#resolveDrainIfReady();
  }

  async #releaseSession(frame: NativeProviderSessionReleaseFrame): Promise<void> {
    await this.#waitForSessionHandlers(frame.sessionId);
    await this.#cleanupSession(frame.sessionId);
    if (this.#sendResults) {
      await this.#serializeOutput(() =>
        this.#sendResults
          ? this.#sendValidated({
              kind: 'native.provider.session_released',
              ...sessionIdentity(frame),
            })
          : Promise.resolve(),
      );
    }
    this.#sessionCleanups.delete(frame.sessionId);
    this.#resolveDrainIfReady();
  }

  async #waitForSessionHandlers(sessionId: string): Promise<void> {
    for (;;) {
      const active = [...this.#invocations.values()]
        .filter((invocation) => invocation.sessionId === sessionId)
        .flatMap((invocation) => (invocation.active ? [invocation.active.settled] : []));
      if (active.length === 0) return;
      await Promise.all(active);
    }
  }

  async #cleanupSession(sessionId: string): Promise<void> {
    try {
      await this.#options.releaseSession(sessionId);
    } catch (error) {
      this.#cleanupFailed = true;
      throw error;
    }
    this.#seenSessions.delete(sessionId);
  }

  async #run(
    frame: NativeProviderSubcallFrame,
    invocation: Invocation,
    active: NonNullable<Invocation['active']>,
  ): Promise<void> {
    try {
      let outcome: NativeCapabilityHandlerOutcome;
      try {
        outcome = await this.#options.handle(frame, {
          signal: active.controller.signal,
        });
      } catch {
        outcome = { ok: false, code: 'operation_failed' };
      }
      if (!this.#sendResults) return;
      await this.#serializeOutput(async () => {
        if (!this.#sendResults) return;
        const identity = {
          hostEpoch: this.#transport.hostEpoch,
          operationId: frame.operationId,
          subcallId: frame.subcallId,
          ordinal: frame.ordinal,
          bindingId: frame.bindingId,
        };
        if (!outcome.ok) {
          await this.#sendValidated({
            kind: 'native.provider.result',
            ...identity,
            ok: false,
            error: { code: outcome.code },
          });
          return;
        }
        let ref: NativeCapabilityAttachmentRef | undefined;
        if (outcome.attachment) ref = await this.#sendAttachment(identity, outcome.attachment);
        if (!this.#sendResults) return;
        const result = outcome.complete(ref);
        if (result.kind !== frame.subcall.kind) {
          throw new Error('Native capability result kind does not match the subcall');
        }
        requireMatchingAttachment(result, ref);
        await this.#sendValidated({
          kind: 'native.provider.result',
          ...identity,
          ok: true,
          result,
        });
      });
    } catch (error) {
      if (this.#sendResults) this.#failTransport(asError(error));
    } finally {
      if (invocation.active === active) {
        invocation.active = undefined;
        invocation.nextOrdinal += 1;
      }
      active.resolveSettled();
      this.#resolveDrainIfReady();
    }
  }

  async #sendAttachment(
    identity: Omit<NativeProviderChunkFrame, 'kind' | 'attachmentId' | 'index' | 'data'>,
    attachment: NativeCapabilityAttachment,
  ): Promise<NativeCapabilityAttachmentRef> {
    const bytes = Buffer.from(attachment.data);
    if (bytes.byteLength === 0 || bytes.byteLength > NATIVE_PROVIDER_MAX_ATTACHMENT_BYTES) {
      throw new Error('Native capability result attachment limit exceeded');
    }
    const ref = {
      attachmentId: attachment.attachmentId,
      byteLength: bytes.byteLength,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      mimeType: attachment.mimeType,
    } satisfies NativeCapabilityAttachmentRef;
    for (let offset = 0, index = 0; offset < bytes.byteLength; index += 1) {
      if (!this.#sendResults) break;
      const chunk = bytes.subarray(offset, offset + this.#chunkBytes);
      offset += chunk.byteLength;
      await this.#sendValidated({
        kind: 'native.provider.chunk',
        ...identity,
        attachmentId: attachment.attachmentId,
        index,
        data: chunk.toString('base64'),
      });
    }
    return ref;
  }

  #serializeOutput(write: () => Promise<void>): Promise<void> {
    const result = this.#writeTail.then(write);
    this.#writeTail = result.catch(() => undefined);
    return result;
  }

  #sendValidated(
    frame:
      | NativeProviderChunkFrame
      | NativeProviderResultFrame
      | NativeProviderSessionReleasedFrame,
  ): Promise<void> {
    return this.#transport.send(decodeNativeProviderClientFrame(frame));
  }

  #requireEpoch(hostEpoch: string): void {
    if (hostEpoch !== this.#transport.hostEpoch)
      throw new Error('Native Provider frame belongs to a different Host Epoch');
  }

  #requireInvocation(operationId: string, bindingId: string): Invocation {
    const invocation = this.#invocations.get(operationId);
    if (!invocation || invocation.bindingId !== bindingId) {
      throw new Error('Runtime Host referenced an unmatched Native Provider invocation');
    }
    return invocation;
  }

  #resolveDrainIfReady(): void {
    if (
      this.#admissionOpen ||
      this.#drainStarted ||
      [...this.#invocations.values()].some((item) => item.active)
    ) {
      return;
    }
    this.#drainStarted = true;
    this.#invocations.clear();
    const cleanup = (async () => {
      const tasks = [...this.#seenSessions].map((sessionId) => {
        const existing = this.#sessionCleanups.get(sessionId);
        if (existing) return existing.task;
        const task = Promise.resolve().then(() => this.#cleanupSession(sessionId));
        this.#sessionCleanups.set(sessionId, { releaseId: '', task });
        return task;
      });
      const outcomes = await Promise.allSettled(tasks);
      const failure = outcomes.find(
        (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected',
      );
      if (failure) this.#failTransport(asError(failure.reason));
    })();
    void cleanup.finally(() => this.#resolveDrained());
  }

  #failTransport(error: Error): void {
    if (this.#transportFailed) return;
    this.#transportFailed = true;
    this.#admissionOpen = false;
    this.#sendResults = false;
    for (const invocation of this.#invocations.values()) invocation.active?.controller.abort();
    this.#transport.fail(error);
    this.#resolveDrainIfReady();
  }
}

function settlement(): Pick<NonNullable<Invocation['active']>, 'settled' | 'resolveSettled'> {
  let resolveSettled!: () => void;
  const settled = new Promise<void>((resolve) => {
    resolveSettled = resolve;
  });
  return { settled, resolveSettled };
}

function sessionIdentity(frame: NativeProviderSessionReleaseFrame) {
  return {
    hostEpoch: frame.hostEpoch,
    registrationId: frame.registrationId,
    releaseId: frame.releaseId,
    sessionId: frame.sessionId,
  };
}

function requireMatchingAttachment(
  result: NativeProviderResultPayload,
  ref?: NativeProviderAttachmentRef,
): void {
  const refs = nativeProviderResultAttachmentRefs(result);
  if (refs.length !== (ref ? 1 : 0) || (ref && refs[0] !== ref)) {
    throw new Error('Native capability result attachment metadata does not match its chunks');
  }
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error('Native Provider operation failed');
}
