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

export interface QueryResponse {
  answerComponents?: AnswerComponent[];
  sources: Source[];
  confidence: ConfidenceLevel;
  errors?: string[];
}

export interface AnswerComponent {
  type: "text" | "citation";
  content?: string;
  citation?: CitationData;
}

export interface CitationData {
  documentId: string;
  filename: string;
  page: number;
}

export type ConfidenceLevel = "HIGH" | "MEDIUM" | "LOW";

export interface Source {
  documentId: string;
  filename: string;
  page: number;
  content: string;
  score: number;
}
