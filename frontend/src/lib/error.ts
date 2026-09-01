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
