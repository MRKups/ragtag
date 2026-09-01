export {};

// The module uses Svelte runes. Install this no-op runtime shim before importing it.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

// The shim must exist before this Svelte rune module evaluates.
const { ErrorDialogState } = await import('./error-dialog.svelte.ts');

const dialog = new ErrorDialogState();
const activeTitle = () => dialog.active?.title;
const queuedCount = () => dialog.queue.length;

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
if (dialog.active || queuedCount() !== 0) {
  throw new Error('Clear must remove the active and queued notices for a recovered source.');
}
