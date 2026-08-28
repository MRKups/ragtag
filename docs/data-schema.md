# Data Schema

The data layer uses a local SQLite file for relational data and LanceDB (an embedded, native Rust vector database) for vector storage.

## SQLite (`app.db`)

Stored in the OS Application Data directory (e.g., `~/.config/rag-demo/app.db`).

### `documents` Table

The single source of truth for file registry and processing state.

```sql
CREATE TABLE documents (
    id TEXT PRIMARY KEY,                       -- UUID v7 (time-sortable)
    
    -- File info
    filename TEXT NOT NULL,
    file_type TEXT NOT NULL,                   -- 'pdf'
    file_size_bytes BIGINT,
    file_hash BLOB,                            -- SHA-512 hash stored as raw bytes (not hex string) to prevent duplicates
    storage_path TEXT NOT NULL,                -- Relative path inside the AppData folder
    
    -- Processing state
    status TEXT NOT NULL DEFAULT 'queued',
    -- 'queued'   : Eligible for/recoverable by the background worker
    -- 'embedding': Background task is actively processing
    -- 'indexed'  : Replacement vectors were stored successfully and are eligible for citations
    -- 'failed'   : Processing error; retryable failures are requeued during startup recovery
    
    -- Indexing metadata
    chunk_count INTEGER,
    embedding_model TEXT,
    embedding_attempts INTEGER DEFAULT 0,
    
    -- Timestamps
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    processing_started_at DATETIME,
    indexed_at DATETIME,
    
    -- Error tracking
    error_message TEXT,
    retry_count INTEGER DEFAULT 0
);

CREATE INDEX idx_documents_status ON documents(status);
CREATE UNIQUE INDEX idx_documents_file_hash ON documents(file_hash);
CREATE INDEX idx_documents_created ON documents(created_at DESC);
```

## LanceDB (Embedded Vector DB)

Stored locally in a `.lancedb` directory alongside the SQLite database.

### Schema / Dataset Layout

LanceDB manages tables using a strongly typed schema. Each row represents a single text chunk from a document.

| Field | Type | Description |
|-------|------|-------------|
| `vector` | `FixedSizeList<Float32>(768)` | The 768-dimensional embedding from the local model (e.g., `EmbeddingGemma-300M`). |
| `document_id` | `String` | Foreign key referencing `documents.id` in SQLite. |
| `page` | `Int32` | 1-indexed page number for citations. |
| `chunk_index` | `Int32` | Position within the document. |
| `content` | `String` | Chunk text stored with its `"passage: "` embedding prefix; reused for RAG prompt synthesis. |

LanceDB does not contain document status, extraction version, or chunking configuration. The application deletes a document's old rows before upserting replacements and only then marks its SQLite row `indexed`. Query code checks SQLite status after LanceDB returns candidates, rather than applying a status filter in the vector query; non-indexed candidates can therefore occupy retrieval slots.

## Application Settings

Instead of `localStorage` (as done in previous web iterations), settings are safely written to an `app_settings` SQLite table managed by Rust. This ensures atomicity alongside document metadata.

```sql
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```
Example keys:
- `inference_mode`: `"local"` or `"external"`.
- `external_base_url`: URL for an OpenAI-compatible endpoint, used in external mode.
- `external_model`: Model name sent to that endpoint; absent values fall back to `"gpt-4o-mini"`.
- `use_query_enhancement`: `"true"` or `"false"`; expands recognized technical abbreviations only for retrieval, leaving the user prompt unchanged.
- `generation_model`: Hugging Face repo ID used for local GGUF download (for example, `"unsloth/Qwen3-1.7B-GGUF"`).
- `gguf_filename`: Filename of the GGUF file within the generation-model repo (for example, `"Qwen3-1.7B-Q4_K_M.gguf"`).
- `chunk_size`: Estimated-token character-window size (default `"512"`).
- `chunk_overlap`: Estimated-token overlap (default `"80"`).
- `extraction_version`: Internal global marker used to requeue indexed documents after an extraction change.

`chunk_size` and `chunk_overlap` are validated together when a setting changes, but changing either setting does not itself requeue already indexed documents. They are character estimates (four characters per estimated token), not exact tokenizer settings.

*Note:* Sensitive data like the `external_api_key` for BYO inference must **never** be stored in SQLite. Instead, it is managed securely in the OS keychain using the `keyring` crate.

## IPC Data Types

These types define the shape of data sent from Rust to the frontend, specifically for the LLM streaming responses via Tauri v2 Channels.

```rust
// In crates/domain/src/events.rs
#[derive(serde::Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenEvent {
    Token   { text: String },
    Done    { citations: Vec<Citation> },
    Error   { message: String, citations: Vec<Citation> },
}

#[derive(serde::Serialize, Clone)]
pub struct Citation {
    pub document_id: String,
    pub filename:    String,
    pub storage_path: String,
    pub page:        i32,
    pub chunk_content: String,
    pub score:       f32,
}

```
`TokenEvent::Error` is used for failures after a response stream begins. Failures before streaming—for example retrieval failures or an external request/status error—reject the `execute_query` IPC command instead.
