import type { IngestionProgress } from '$lib/types';

export class DocumentProgressState {
  byDocument = $state<Record<string, IngestionProgress>>({});

  record(progress: IngestionProgress) {
    this.byDocument = {
      ...this.byDocument,
      [progress.document_id]: progress,
    };
  }

  get(documentId: string): IngestionProgress | undefined {
    return this.byDocument[documentId];
  }

  remove(documentId: string) {
    if (!(documentId in this.byDocument)) return;

    const { [documentId]: _, ...remaining } = this.byDocument;
    this.byDocument = remaining;
  }
}

export const documentProgressState = new DocumentProgressState();
