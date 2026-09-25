// Normalizes typed IPC errors and classifies transient failures for user guidance.
import type { AppErrorPayload, ErrorCode } from './types.ts';

const TRANSIENT_CODES: ReadonlySet<ErrorCode> = new Set([
  'queue_full',
  'inference',
  'storage',
]);

export function errorMessage(error: unknown): string {
  if (
    typeof error === 'object'
    && error !== null
    && 'message' in error
    && typeof error.message === 'string'
  ) {
    return error.message;
  }

  return error instanceof Error ? error.message : String(error);
}

export function isAppError(error: unknown): error is AppErrorPayload {
  return (
    typeof error === 'object'
    && error !== null
    && 'code' in error
    && typeof (error as Record<string, unknown>).code === 'string'
    && 'operation' in error
    && 'detail' in error
    && 'recovery' in error
  );
}

export function isTransientError(error: unknown): boolean {
  if (isAppError(error)) {
    return TRANSIENT_CODES.has(error.code);
  }
  return false;
}

export function toAppErrorPayload(error: unknown, fallbackOperation = 'Operation'): AppErrorPayload {
  if (isAppError(error)) {
    return error;
  }
  const message = errorMessage(error);
  return {
    code: 'internal',
    operation: fallbackOperation,
    detail: message,
    recovery: 'Check application logs or retry the action.',
    message: `${fallbackOperation}: ${message}. Check application logs or retry the action.`,
  };
}

