export {};

// The module uses Svelte runes. Install this no-op runtime shim before importing it.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

// The shim must exist before this Svelte rune module evaluates.
const { ErrorDialogState } = await import('./error-dialog.svelte.ts');

const dialog = new ErrorDialogState();
const activeTitle = () => dialog.active?.title;
const queuedCount = () => dialog.queue.length;
const currentNotice = () => dialog.active;

dialog.report({ dedupeKey: 'upload', title: 'Upload rejected', message: 'Invalid source' });
dialog.report({ dedupeKey: 'upload', title: 'Duplicate', message: 'Must be ignored' });
dialog.report({ dedupeKey: 'lifecycle', title: 'Model unavailable', message: 'Retry later' });

if (activeTitle() !== 'Upload rejected' || queuedCount() !== 1) {
  throw new Error('Error dialogs must deduplicate a source while preserving distinct notices.');
}

dialog.dismiss();
if (activeTitle() !== 'Model unavailable' || queuedCount() !== 0) {
  throw new Error('Dismiss must advance to exactly one distinct queued notice.');
}

dialog.clear('lifecycle');
if (currentNotice() || queuedCount() !== 0) {
  throw new Error('Clear must remove the active and queued notices for a recovered source.');
}

// Test reportError with structured AppErrorPayload
dialog.reportError({
  code: 'vector_db',
  operation: 'Index compaction',
  detail: 'Disk full',
  recovery: 'Free up disk space and retry.',
  message: 'Index compaction: Disk full. Free up disk space and retry.'
}, 'Compaction Failed');

if (currentNotice()?.code !== 'vector_db' || currentNotice()?.title !== 'Index compaction' || currentNotice()?.guidance !== 'Free up disk space and retry.') {
  throw new Error('reportError did not correctly map structured AppError properties.');
}

dialog.dismiss();

// Test reportError with native Error fallback
dialog.reportError(new Error('Unexpected network glitch'), 'Network Request');
if (currentNotice()?.code !== 'internal' || currentNotice()?.title !== 'Network Request' || !currentNotice()?.message.includes('Unexpected network glitch')) {
  throw new Error('reportError did not correctly map fallback error properties.');
}

