import type { Document, IngestionProgress } from '$lib/types';
import { errorDialog } from './error-dialog.svelte.ts';

export class DocumentProgressState {
  byDocument = $state<Record<string, IngestionProgress>>({});

  private terminalDocumentIds = new Set<string>();

  record(progress: IngestionProgress) {
    if (this.terminalDocumentIds.has(progress.document_id)) {
      this.remove(progress.document_id);
      return;
    }

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

  reconcile(documents: Iterable<Pick<Document, 'id' | 'status'>>) {
    for (const document of documents) {
      if (document.status === 'indexed' || document.status === 'failed') {
        this.finalize(document.id);
      } else {
        this.terminalDocumentIds.delete(document.id);
      }
    }
  }

  finalize(documentId: string) {
    this.terminalDocumentIds.add(documentId);
    this.remove(documentId);
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
