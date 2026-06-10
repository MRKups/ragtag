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
    -- 'queued'   : Waiting in the tokio mpsc channel
    -- 'embedding': Background task is actively processing
    -- 'indexed'  : Vectors saved, ready for search
    -- 'failed'   : Processing error
    
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
| `content` | `String` | The raw chunk text for RAG prompt synthesis. |

## Application Settings

Instead of `localStorage` (as done in previous web iterations), settings are safely written to an `app_settings` SQLite table managed by Rust. This ensures atomicity alongside document metadata.

```sql
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```
Example keys:
- `setup_state`: Tracking first-run downloads (`'embedder_pending'`, `'embedder_ready'`, `'llm_pending'`, `'ready'`).
- `inference_mode`: `"local"` or `"external"`
- `external_base_url`: URL for an OpenAI-compatible endpoint (used if mode is external).
- `use_query_enhancement`: `"true"` or `"false"`
- `embedding_model`: `"EmbeddingGemma-300M"`
- `generation_model`: HF repo ID consumed by `mistral.rs` (e.g., `"unsloth/Qwen3-1.7B-GGUF"`).
- `gguf_filename`: Filename of the GGUF file within the generation model repo (e.g., `"Qwen3-1.7B-Q4_K_M.gguf"`).
- `vector_dim`: `"768"`
- `chunk_size`: `"512"`
- `chunk_overlap`: `"80"`

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
