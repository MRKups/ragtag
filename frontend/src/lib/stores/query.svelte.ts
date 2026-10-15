// Manages interactive query state, streamed token accumulation, and retrieval citations.
import type { Citation } from '../types';
import type { DocumentSelection } from '../citations';

export class QueryState {
  text = $state('');
  citations = $state<Citation[]>([]);
  retrievalWarning = $state<string | null>(null);
  queryText = $state('');
  submittedQuery = $state('');
  selectedDocument = $state<DocumentSelection | null>(null);
  currentRequestId = $state<string | null>(null);

  get isLoading() {
    return this.currentRequestId !== null;
  }

  get isStreaming() {
    return this.isLoading;
  }

  stopGenerating() {
    this.currentRequestId = null;
  }

  cancel() {
    this.stopGenerating();
  }

  reset() {
    this.stopGenerating();
    this.text = '';
    this.citations = [];
    this.retrievalWarning = null;
    this.queryText = '';
    this.submittedQuery = '';
    this.selectedDocument = null;
  }

  complete(citations: Citation[]) {
    this.citations = citations;
    this.stopGenerating();
    this.queryText = '';
  }
}

export const queryState = new QueryState();
