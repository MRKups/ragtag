import type { IngestionProgress } from '$lib/types';
import { errorDialog } from './error-dialog.svelte';

export class DocumentProgressState {
  byDocument = $state<Record<string, IngestionProgress>>({});

  record(progress: IngestionProgress) {
    this.byDocument = {
      ...this.byDocument,
      [progress.document_id]: progress,
    };

    if (progress.type === 'error') {
      errorDialog.report({
        dedupeKey: `document-processing-${progress.document_id}`,
        title: 'Document processing failed',
        message: 'Ragtag could not finish indexing a document.',
        guidance: 'Review the document status, then remove and upload a new copy if needed.'
      });
    }
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
