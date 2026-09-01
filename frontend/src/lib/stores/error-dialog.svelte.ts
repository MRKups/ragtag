export interface ErrorDialogAction {
  label: string;
  run(): void | Promise<void>;
}

export interface ErrorDialogNotice {
  dedupeKey?: string;
  title: string;
  message: string;
  guidance?: string;
  action?: ErrorDialogAction;
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
