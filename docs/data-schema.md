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
    -- 'deleting' : Durable deletion is in progress; excluded from retrieval and retried at startup
    -- 'failed'   : Processing error; retryable failures are requeued during startup recovery
    
    -- Indexing metadata
    chunk_count INTEGER,
    embedding_model TEXT,
    embedding_attempts INTEGER DEFAULT 0,

    -- Successful-index provenance (set only after vector replacement succeeds)
    indexed_extraction_version TEXT,
    indexed_chunk_size INTEGER,
    indexed_chunk_overlap INTEGER,
    
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
| `content` | `String` | Raw chunk text reused for citations and RAG prompt synthesis. The ingest embedding input prepends `"passage: "` without persisting that instruction prefix. |

LanceDB does not contain document status, extraction version, or chunking configuration. SQLite records the nullable successful-index provenance for each document after replacement vectors are stored, so it remains the registry authority for whether an indexed document matches the desired extraction and chunk settings. First table creation is serialized before concurrent search or ingest access. The application deletes a document's old rows before upserting replacements and only then marks its SQLite row `indexed` with that provenance. FTS is available after rows exist; the default IVF-PQ index is deferred until the shared table has at least 256 vectors, with flat vector search serving smaller stores. Query code checks SQLite status after LanceDB returns candidates, rather than applying a status filter in the vector query; non-indexed candidates can therefore occupy retrieval slots. Hybrid RRF relevance is used for rank ordering but is not a calibrated citation-confidence value.

## Application Settings

Instead of `localStorage` (as done in previous web iterations), settings are safely written to an `app_settings` SQLite table managed by Rust. This ensures atomicity alongside document metadata.

```sql
CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```
Example keys:
- `inference_mode`: `"local"` or `"external"`; selects the default generation route for new queries.
- `external_base_url`: Credential-free HTTP(S) base URL for an OpenAI-compatible endpoint, normalized before `/models` readiness and chat-completions requests.
- `external_model`: Model name required in the endpoint’s `/models` response and sent to that endpoint; absent values fall back to `"gpt-4o-mini"`.
- `use_query_enhancement`: `"true"` or `"false"`; expands recognized technical abbreviations only for retrieval, leaving the user prompt unchanged.
- `generation_model`: Hugging Face repo ID used for local GGUF download. Default: `"unsloth/Qwen3-1.7B-GGUF"`.
- `tokenizer_model`: Hugging Face tokenizer repository compatible with the local GGUF model. Default: `"Qwen/Qwen3-1.7B"`.
- `gguf_filename`: Filename of the GGUF file within the generation-model repo. Default: `"Qwen3-1.7B-Q4_K_M.gguf"`.

Changing `generation_model`, `tokenizer_model`, or `gguf_filename` while `inference_mode` is `"local"` immediately withdraws the loaded local model, emits `llm_unready`, and starts one generation-tagged initialization from the persisted settings. Rapid edits coalesce behind the active initialization, so an obsolete initializer cannot publish progress, readiness, errors, or a model and only the newest persisted configuration is subsequently initialized. While `"external"` is selected, those local settings are stored without downloading or building a local model; switching back to `"local"` uses the stored values.

- `chunk_size`: Token count per embedding-safe chunk (default `"512"`, maximum `"2048"`).
- `chunk_overlap`: Token overlap between chunks on the same page (default `"80"`).

`chunk_size` and `chunk_overlap` are validated together when a setting changes. Under the document-operations write lock, the final pair is persisted and all indexed documents with absent or mismatched provenance are atomically returned to `queued`; their saved provenance remains until the successful replacement records the new values. After the transaction and lock release, the command uses `try_send()` for reindex jobs: a full queue returns typed `QueueFull` immediately while a background producer awaits capacity for every remaining durable queued job. Chunks are split and counted with the active embedding tokenizer, including the `passage: ` prefix and special tokens, so no stored chunk is silently truncated during embedding.

The Settings UI keeps local and external configuration independent. Its header reports the active inference route from `inference_mode`; the Local model and Remote provider cards form an accessible radio group that persists that route and each opens its own configuration modal. The Local model modal initializes absent local settings with the defaults above and can restore those three local settings without changing `inference_mode` or the external configuration.

*Note:* Sensitive data like the `external_api_key` for BYO inference must **never** be stored in SQLite. Instead, it is managed securely in the OS keychain using the `keyring` crate.

## IPC Data Types

These types define the shape of data sent from Rust to the frontend, specifically for the LLM streaming responses via Tauri v2 Channels.

```rust
// In crates/domain/src/events.rs
#[derive(serde::Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenEvent {
    Token   { text: String },
    Warning { message: String },
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
    pub is_hybrid:   bool,
}

```

`Citation::is_hybrid` is `true` when LanceDB used reciprocal-rank fusion. The UI must not treat its rank-derived `score` as calibrated confidence; it displays confidence only for vector-only retrieval.

`TokenEvent::Warning` reports a non-fatal retrieval degradation, such as fallback to vector-only search. `TokenEvent::Error` is used for failures after a response stream begins. Failures before streaming—including no usable retrieved excerpts, retrieval failures, or an external request/status error—reject the `execute_query` IPC command instead.
