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

export function isAppError(error: unknown): error is import('./types.ts').AppErrorPayload {
  return (
    typeof error === 'object'
    && error !== null
    && 'kind' in error
    && typeof (error as Record<string, unknown>).kind === 'string'
  );
}

