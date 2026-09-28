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

const queryToCancel = new QueryState();
queryToCancel.currentRequestId = 'request-2';
queryToCancel.isLoading = true;
queryToCancel.isStreaming = true;
queryToCancel.text = 'Partial generated tokens';
queryToCancel.citations = [
  {
    document_id: 'document-2',
    filename: 'manual.pdf',
    storage_path: 'documents/document-2.pdf',
    page: 2,
    chunk_content: 'Excerpt',
    score: 0.8,
    is_hybrid: false
  }
];

queryToCancel.cancel();

if (
  queryToCancel.currentRequestId !== null
  || queryToCancel.isLoading
  || queryToCancel.isStreaming
  || queryToCancel.text !== 'Partial generated tokens'
  || queryToCancel.citations.length !== 1
) {
  throw new Error('Cancelling an in-flight query must stop generation but retain partial text and citations.');
}

queryToCancel.reset();
const resetQuery = queryToCancel as { text: string; citations: unknown[]; submittedQuery: string };
if (resetQuery.text !== '' || resetQuery.citations.length !== 0 || resetQuery.submittedQuery !== '') {
  throw new Error('Reset must clear text, citations, and submittedQuery.');
}

const queryPreserve = new QueryState();
queryPreserve.submittedQuery = 'What is the torque spec?';
queryPreserve.complete([]);
if (queryPreserve.submittedQuery !== 'What is the torque spec?') {
  throw new Error('A completed answer must preserve submittedQuery.');
}

