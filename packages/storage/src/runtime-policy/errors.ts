export type RuntimePolicyStoreErrorCode =
  | 'invalid_document'
  | 'invalid_policy_input'
  | 'invalid_connection_input'
  | 'invalid_credential_input'
  | 'io_failed'
  | 'commit_outcome_unknown';

export class RuntimePolicyStoreError extends Error {
  constructor(
    readonly code: RuntimePolicyStoreErrorCode,
    message: string,
    options?: ErrorOptions,
  ) {
    super(message, options);
    this.name = 'RuntimePolicyStoreError';
  }
}

export type CodecSource =
  | 'invalid_document'
  | 'invalid_policy_input'
  | 'invalid_connection_input'
  | 'invalid_credential_input';

export function codecError(source: CodecSource, message: string): RuntimePolicyStoreError {
  return new RuntimePolicyStoreError(source, message);
}

export function invalidDocument(message: string, cause?: unknown): RuntimePolicyStoreError {
  return new RuntimePolicyStoreError(
    'invalid_document',
    message,
    cause === undefined ? undefined : { cause },
  );
}

export function ioFailed(message: string, cause: unknown): RuntimePolicyStoreError {
  return new RuntimePolicyStoreError('io_failed', message, { cause });
}

export function commitOutcomeUnknown(message: string, cause: unknown): RuntimePolicyStoreError {
  return new RuntimePolicyStoreError('commit_outcome_unknown', message, { cause });
}
