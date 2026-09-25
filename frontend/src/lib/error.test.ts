import { errorMessage, isAppError, isTransientError, toAppErrorPayload } from './error.ts';
import type { AppErrorPayload } from './types.ts';

const structuredError: AppErrorPayload = {
  code: 'inference',
  operation: 'Query execution',
  detail: 'No usable document excerpts were retrieved',
  recovery: 'Try indexing documents with relevant content first.',
  message: 'Query execution: No usable document excerpts were retrieved. Try indexing documents with relevant content first.',
};

if (!errorMessage(structuredError).includes('No usable document excerpts were retrieved')) {
  throw new Error('Structured AppError payload message was not extracted.');
}

if (!errorMessage({ kind: 'Inference', message: 'Legacy inference error' }).includes('Legacy inference error')) {
  throw new Error('Legacy error message was not extracted.');
}

if (errorMessage(new Error('fallback error')) !== 'fallback error') {
  throw new Error('Native Error fallback was not preserved.');
}

if (errorMessage('plain string error') !== 'plain string error') {
  throw new Error('Plain string fallback was not preserved.');
}

if (!isAppError(structuredError)) {
  throw new Error('isAppError must return true for structured AppErrorPayload.');
}

if (isAppError(new Error('native error')) || isAppError({ kind: 'Inference', message: 'missing code' })) {
  throw new Error('isAppError must return false for non-structured errors.');
}

if (!isTransientError(structuredError)) {
  throw new Error('isTransientError must identify inference errors as transient.');
}

const queueFullError: AppErrorPayload = {
  code: 'queue_full',
  operation: 'Document upload',
  detail: 'Processing queue full',
  recovery: 'Wait a moment and try again.',
  message: 'Document upload: Processing queue full. Wait a moment and try again.',
};
if (!isTransientError(queueFullError)) {
  throw new Error('isTransientError must identify queue_full as transient.');
}

const invalidInputError: AppErrorPayload = {
  code: 'invalid_input',
  operation: 'Document upload',
  detail: 'Unsupported file extension',
  recovery: 'Select a PDF, DOCX, TXT, or MD file.',
  message: 'Document upload: Unsupported file extension. Select a PDF, DOCX, TXT, or MD file.',
};
if (isTransientError(invalidInputError)) {
  throw new Error('isTransientError must identify invalid_input as non-transient.');
}

const converted = toAppErrorPayload(new Error('Connection dropped'), 'Network test');
if (converted.code !== 'internal' || converted.operation !== 'Network test' || !converted.detail.includes('Connection dropped')) {
  throw new Error('toAppErrorPayload failed to convert native error.');
}

const passthrough = toAppErrorPayload(structuredError, 'Ignored fallback');
if (passthrough !== structuredError) {
  throw new Error('toAppErrorPayload must pass through existing AppErrorPayload untouched.');
}
