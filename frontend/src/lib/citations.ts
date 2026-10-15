import type { Citation } from './types';

export interface DocumentSelection {
  documentId: string;
  filename: string;
  storagePath?: string;
  page: number;
}

export interface CitationSelection extends DocumentSelection {
  storagePath: string;
}

export function citationSelection(citation: Citation): CitationSelection {
  return {
    filename: citation.filename,
    storagePath: citation.storage_path,
    documentId: citation.document_id,
    page: citation.page
  };
}
