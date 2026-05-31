export type DocumentStatus = "queued" | "embedding" | "indexed" | "failed";

export interface Document {
  id: string;
  filename: string;
  file_type: string;
  file_size_bytes?: number;
  file_hash?: number[]; // Vec<u8> serialized
  storage_path: string;
  status: DocumentStatus;
  chunk_count?: number;
  embedding_model?: string;
  embedding_attempts: number;
  created_at?: string;
  processing_started_at?: string;
  indexed_at?: string;
  error_message?: string;
  retry_count: number;
}

export interface Citation {
  document_id: string;
  filename: string;
  page: number;
  chunk_content: string;
}

export type TokenEvent = 
  | { type: "token"; text: string }
  | { type: "done"; citations: Citation[] }
  | { type: "error"; message: string };

export interface AppError {
  kind: string;
  message: string;
}


