export type Platform = "macos" | "windows" | "linux";

export type DocumentStatus = "queued" | "embedding" | "indexed" | "failed" | "deleting";

export interface IndexProvenance {
  extraction_version: string;
  chunk_size: number;
  chunk_overlap: number;
}

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
  indexing_provenance?: IndexProvenance;
  error_message?: string;
  retry_count: number;
}

export type DocumentPreview =
  | { kind: 'plain_text'; content: string; truncated: boolean }
  | { kind: 'markdown'; blocks: MarkdownBlock[]; truncated: boolean };

export type MarkdownBlock =
  | { kind: 'heading'; level: number; content: MarkdownInline[] }
  | { kind: 'paragraph'; content: MarkdownInline[] }
  | { kind: 'block_quote'; blocks: MarkdownBlock[] }
  | { kind: 'list'; start: number | null; items: MarkdownBlock[][] }
  | { kind: 'code_block'; language: string | null; content: string }
  | { kind: 'table'; header: MarkdownInline[][]; rows: MarkdownInline[][][] }
  | { kind: 'thematic_break' };

export type MarkdownInline =
  | { kind: 'text'; content: string }
  | { kind: 'code'; content: string }
  | { kind: 'emphasis'; content: MarkdownInline[] }
  | { kind: 'strong'; content: MarkdownInline[] }
  | { kind: 'strikethrough'; content: MarkdownInline[] }
  | { kind: 'link'; href: string; content: MarkdownInline[] }
  | { kind: 'break' };

export interface Citation {
  document_id: string;
  filename: string;
  storage_path: string;
  page: number;
  chunk_content: string;
  score: number;
  is_hybrid: boolean;
}

export type TokenEvent =
  | { type: "token"; text: string }
  | { type: "warning"; message: string }
  | { type: "done"; citations: Citation[] }
  | { type: "error"; message: string; citations: Citation[] };

export type ErrorCode =
  | 'not_found'
  | 'invalid_input'
  | 'queue_full'
  | 'duplicate_document'
  | 'storage'
  | 'vector_db'
  | 'inference'
  | 'internal';

export interface AppErrorPayload {
  code: ErrorCode;
  operation: string;
  detail: string;
  recovery: string;
  message: string;
}

export interface ExternalProviderConnection {
  models: string[];
}

export type ApiKeyStorage = 'keychain' | 'database' | 'none';

export interface ApiKeyStatus {
  has_key: boolean;
  storage: ApiKeyStorage;
}


export type IngestionProgress =
  | {
      type: "extraction";
      document_id: string;
      current_page: number;
      total_pages: number;
    }
  | {
      type: "chunking";
      document_id: string;
      current_page: number;
      total_pages: number;
      chunks_on_page: number;
      total_chunks: number;
    }
  | {
      type: "embedding";
      document_id: string;
      current_batch: number;
      total_batches: number;
      embedded_chunks: number;
      total_chunks: number;
    }
  | {
      type: "stored";
      document_id: string;
      chunk_count: number;
    }
  | {
      type: "error";
      document_id: string;
      message: string;
    };

export type DownloadStageType =
  | 'fetching_info'
  | 'downloading'
  | 'complete'
  | 'loading_model'
  | 'error';

export interface ModelDownloadProgress {
  model_id: string;
  current_file: string;
  file_downloaded: number;
  file_total: number;
  files_completed: number;
  total_files: number;
  type: DownloadStageType;
  message?: string;
}

export interface CapacityState {
  ordinary_available: number;
  ordinary_capacity: number;
  protected_available: number;
  protected_capacity: number;
}

export interface AppLifecycleState {
  embedder_ready: boolean;
  llm_ready: boolean;
  inference_mode: string;
  model_download_active: boolean;
  model_download_progress: ModelDownloadProgress | null;
  active_queries_count: number;
  ingestion_active: boolean;
  active_document_id: string | null;
  capacity: CapacityState;
}


