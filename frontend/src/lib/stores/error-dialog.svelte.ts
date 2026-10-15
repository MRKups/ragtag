import type { ErrorCode } from '../types';
import { isTransientError, toAppErrorPayload } from '../error.ts';

export interface ErrorDialogAction {
  label: string;
  run(): void | Promise<void>;
}

export interface ErrorDialogNotice {
  dedupeKey?: string;
  title: string;
  message: string;
  guidance?: string;
  code?: ErrorCode;
  transient?: boolean;
  action?: ErrorDialogAction;
}

export function documentProcessingFailureNotice(documentId: string, action?: ErrorDialogAction): ErrorDialogNotice {
  return {
    dedupeKey: `document-processing-${documentId}`,
    title: 'Document processing failed',
    message: 'Ragtag could not finish indexing a document.',
    guidance: 'Review the document status, then remove and upload a new copy if needed.',
    ...(action ? { action } : {})
  };
}

export class ErrorDialogState {
  active = $state<ErrorDialogNotice | null>(null);
  queue = $state<ErrorDialogNotice[]>([]);

  report(notice: ErrorDialogNotice) {
    if (notice.dedupeKey && this.#hasDedupeKey(notice.dedupeKey)) return;

    if (this.active) {
      this.queue.push(notice);
    } else {
      this.active = notice;
    }
  }

  reportError(
    error: unknown,
    fallbackTitle = 'Operation Failed',
    action?: ErrorDialogAction,
    dedupeKey?: string,
  ) {
    const payload = toAppErrorPayload(error, fallbackTitle);
    this.report({
      dedupeKey,
      title: payload.operation || fallbackTitle,
      message: payload.detail,
      guidance: payload.recovery,
      code: payload.code,
      transient: isTransientError(error),
      action,
    });
  }

  dismiss() {
    this.active = this.queue.shift() ?? null;
  }

  clear(dedupeKey: string) {
    const remaining = this.queue.filter((notice) => notice.dedupeKey !== dedupeKey);

    if (this.active?.dedupeKey === dedupeKey) {
      this.active = remaining.shift() ?? null;
    }

    this.queue = remaining;
  }

  #hasDedupeKey(dedupeKey: string) {
    return this.active?.dedupeKey === dedupeKey
      || this.queue.some((notice) => notice.dedupeKey === dedupeKey);
  }
}

export const errorDialog = new ErrorDialogState();
