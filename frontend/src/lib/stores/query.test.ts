import type { Citation } from '../types';
import { citationSelection } from '../citations.ts';

// The module uses Svelte runes. Install this no-op runtime shim before importing it.
const runtime = globalThis as { $state?: <T>(value: T) => T };
runtime.$state = <T>(value: T) => value;

// The shim must exist before this Svelte rune module evaluates.
const { QueryState } = await import('./query.svelte.ts');

function expectActivity(query: InstanceType<typeof QueryState>, active: boolean) {
  if (query.isLoading !== active || query.isStreaming !== active) {
    throw new Error('Loading and streaming must both reflect whether a request ID exists.');
  }
}

const citation: Citation = {
  document_id: 'document-1',
  filename: 'notes.txt',
  storage_path: 'documents/document-1.txt',
  page: 1,
  chunk_content: 'Notes',
  score: 0.9,
  is_hybrid: true
};
const selection = citationSelection(citation);
if (
  selection.documentId !== citation.document_id
  || selection.filename !== citation.filename
  || selection.storagePath !== citation.storage_path
  || selection.page !== citation.page
  || Object.keys(selection).length !== 4
) {
  throw new Error('Citation selection must map only the original document identity, path, filename, and retrieved page.');
}
const unmappedCitation = { ...citation, storage_path: '', page: 0 };
const unmappedSelection = citationSelection(unmappedCitation);
if (unmappedSelection.storagePath !== '' || unmappedSelection.page !== 0) {
  throw new Error('Citation mapping must preserve values without adding fallback paths or model-generated pages.');
}
selection.filename = 'Changed selection';
if (citation.filename !== 'notes.txt' || citationSelection(citation).filename !== 'notes.txt') {
  throw new Error('Citation selections must not mutate source evidence or share mutable result objects.');
}

const query = new QueryState();
expectActivity(query, false);
query.queryText = 'Summarize the document';
query.submittedQuery = 'Summarize the document';
query.text = 'Generated answer';
query.retrievalWarning = 'Vector-only retrieval';
query.selectedDocument = citationSelection(citation);
query.currentRequestId = 'request-1';
expectActivity(query, true);

query.complete([citation]);
expectActivity(query, false);

if (
  query.queryText !== ''
  || query.currentRequestId !== null
  || query.isLoading
  || query.isStreaming
  || query.citations.length !== 1
  || query.citations[0] !== citation
  || query.text !== 'Generated answer'
  || query.submittedQuery !== 'Summarize the document'
  || query.retrievalWarning !== 'Vector-only retrieval'
  || query.selectedDocument?.documentId !== citation.document_id
) {
  throw new Error('A completed answer must clear the composer and request while preserving the submitted question and evidence.');
}

const queryToCancel = new QueryState();
queryToCancel.currentRequestId = 'request-2';
expectActivity(queryToCancel, true);
queryToCancel.queryText = 'Editable composer';
queryToCancel.submittedQuery = 'Original question';
queryToCancel.retrievalWarning = 'Retrieval warning';
// Store selections also accept a document without a resolved storage path.
queryToCancel.selectedDocument = { documentId: 'document-2', filename: 'manual.pdf', page: 2 };
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
expectActivity(queryToCancel, false);

if (
  queryToCancel.currentRequestId !== null
  || queryToCancel.isLoading
  || queryToCancel.isStreaming
  || queryToCancel.text !== 'Partial generated tokens'
  || queryToCancel.citations.length !== 1
  || queryToCancel.queryText !== 'Editable composer'
  || queryToCancel.submittedQuery !== 'Original question'
  || queryToCancel.retrievalWarning !== 'Retrieval warning'
  || queryToCancel.selectedDocument?.documentId !== 'document-2'
) {
  throw new Error('Cancelling an in-flight query must retain the composer, submitted question, and partial evidence.');
}

queryToCancel.cancel();
expectActivity(queryToCancel, false);
queryToCancel.currentRequestId = 'request-to-reset';
expectActivity(queryToCancel, true);
queryToCancel.reset();
expectActivity(queryToCancel, false);
const resetQuery: InstanceType<typeof QueryState> = queryToCancel;
if (
  resetQuery.currentRequestId !== null
  || resetQuery.text !== ''
  || resetQuery.citations.length !== 0
  || resetQuery.queryText !== ''
  || resetQuery.submittedQuery !== ''
  || resetQuery.retrievalWarning !== null
  || resetQuery.selectedDocument !== null
) {
  throw new Error('Reset must stop the request and clear the composer, submitted question, and all evidence.');
}

const queryPreserve = new QueryState();
queryPreserve.submittedQuery = 'What is the torque spec?';
queryPreserve.complete([]);
if (queryPreserve.submittedQuery !== 'What is the torque spec?') {
  throw new Error('A completed answer must preserve submittedQuery.');
}

for (const streamStarted of [false, true]) {
  const failedQuery = new QueryState();
  failedQuery.currentRequestId = 'failed-request';
  failedQuery.queryText = 'Composer after failure';
  failedQuery.submittedQuery = 'Submitted before failure';
  failedQuery.text = streamStarted ? 'Partial answer' : '';
  failedQuery.retrievalWarning = 'Retrieval warning';
  failedQuery.selectedDocument = citationSelection(citation);
  // The channel's error branch replaces citations; command rejection does not.
  if (streamStarted) failedQuery.citations = [citation];
  failedQuery.stopGenerating();
  failedQuery.stopGenerating();
  expectActivity(failedQuery, false);
  if (
    failedQuery.currentRequestId !== null
    || failedQuery.queryText !== 'Composer after failure'
    || failedQuery.submittedQuery !== 'Submitted before failure'
    || failedQuery.text !== (streamStarted ? 'Partial answer' : '')
    || failedQuery.citations.length !== (streamStarted ? 1 : 0)
    || failedQuery.retrievalWarning !== 'Retrieval warning'
    || failedQuery.selectedDocument?.documentId !== citation.document_id
  ) {
    throw new Error('Stopping a failed request must retain its composer, submitted question, and available evidence.');
  }
  failedQuery.currentRequestId = 'next-request';
  expectActivity(failedQuery, true);
  failedQuery.currentRequestId = null;
  expectActivity(failedQuery, false);
}
