export {};

// The module uses Svelte runes. Install this no-op runtime shim before importing it.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

const { DocumentProgressState } = await import('./document-progress.svelte.ts');

const progress = new DocumentProgressState();
progress.record({
  type: 'extraction',
  document_id: 'document-1',
  current_page: 1,
  total_pages: 1
});
progress.finalize('document-1');
progress.record({
  type: 'stored',
  document_id: 'document-1',
  chunk_count: 4
});

if (progress.get('document-1') !== undefined) {
  throw new Error('A delayed channel callback must not restore finalized document progress.');
}

progress.record({
  type: 'embedding',
  document_id: 'document-2',
  current_batch: 1,
  total_batches: 1,
  embedded_chunks: 4,
  total_chunks: 4
});
progress.reconcile([{ id: 'document-2', status: 'indexed' }]);

if (progress.get('document-2') !== undefined) {
  throw new Error('A terminal registry status must clear cached document progress.');
}
