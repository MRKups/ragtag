import type { Citation } from '../types';

export class QueryState {
  isLoading = $state(false);
  text = $state('');
  citations = $state<Citation[]>([]);
  error = $state<{ message: string; requestId?: string } | null>(null);
  queryText = $state('');
  selectedDocument = $state<{ filename: string; documentId?: string; page: number } | null>(null);
  isStreaming = $state(false);

  reset() {
    this.isLoading = false;
    this.text = '';
    this.citations = [];
    this.error = null;
    this.queryText = '';
    this.selectedDocument = null;
    this.isStreaming = false;
  }
}

export const queryState = new QueryState();
