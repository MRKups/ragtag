import type { Citation } from '../types';

export class QueryState {
  isLoading = $state(false);
  text = $state('');
  citations = $state<Citation[]>([]);
  retrievalWarning = $state<string | null>(null);
  queryText = $state('');
  selectedDocument = $state<{ documentId: string; filename: string; storagePath?: string; page: number } | null>(null);
  isStreaming = $state(false);
  currentRequestId = $state<string | null>(null);

  reset() {
    this.isLoading = false;
    this.text = '';
    this.citations = [];
    this.retrievalWarning = null;
    this.queryText = '';
    this.selectedDocument = null;
    this.isStreaming = false;
  }

  cancel() {
    this.currentRequestId = null;
    this.reset();
  }

  complete(citations: Citation[]) {
    this.citations = citations;
    this.currentRequestId = null;
    this.isStreaming = false;
    this.isLoading = false;
    this.queryText = '';
  }
}

export const queryState = new QueryState();
