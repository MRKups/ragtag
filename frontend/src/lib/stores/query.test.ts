export {};

// The module uses Svelte runes. Install this no-op runtime shim before importing it.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

// The shim must exist before this Svelte rune module evaluates.
const { QueryState } = await import('./query.svelte.ts');

const query = new QueryState();
query.queryText = 'Summarize the document';
query.currentRequestId = 'request-1';
query.isLoading = true;
query.isStreaming = true;

query.complete([
  {
    document_id: 'document-1',
    filename: 'notes.txt',
    storage_path: 'documents/document-1.txt',
    page: 1,
    chunk_content: 'Notes',
    score: 0.9,
    is_hybrid: true
  }
]);

if (
  query.queryText !== ''
  || query.currentRequestId !== null
  || query.isLoading
  || query.isStreaming
  || query.citations.length !== 1
) {
  throw new Error('A completed answer must clear the composer and preserve its citations.');
}
