---
title: Data schema and IPC contracts
normative-for: [persisted data, settings values, Tauri IPC wire vocabulary]
depends: [app-architecture.md, glossary.md]
status: current reference. Migrations and source code override conflicting statements.
---

# Data schema and IPC contracts

SQLite owns registry state, LanceDB stores derived chunks and vectors, and Tauri IPC carries requests and progress. See [Application architecture](app-architecture.md) for flows.

## SQLite

Migrations define the effective DDL schema.

### `documents`

| Column | Nullability | Meaning |
| --- | --- | --- |
| `id` | primary key | Document identifier. |
| `filename` | required | Original user-visible filename. |
| `file_type` | required | Accepted source type: `pdf`, `docx`, or `text`. |
| `file_size_bytes` | optional | Managed-source byte length. |
| `file_hash` | optional, unique when present | Managed-source content hash used for duplicate detection. |
| `storage_path` | required | Managed-source path relative to application-local data. |
| `status` | required, default `queued` | Document lifecycle state. |
| `chunk_count` | optional | Count recorded after successful indexing. |
| `embedding_model` | optional | Model recorded with successful indexing. |
| `embedding_attempts` | default `0` | Attempts in the current unsuccessful processing run. |
| `indexed_extraction_version` | optional | Extraction version recorded after successful indexing. |
| `indexed_chunk_size` | optional | Chunk size recorded after successful indexing. |
| `indexed_chunk_overlap` | optional | Chunk overlap recorded after successful indexing. |
| `created_at` | default current timestamp | Registry creation time. |
| `processing_started_at` | optional | Most recent processing start time. |
| `indexed_at` | optional | Most recent successful indexing time. |
| `error_message` | optional | Most recent processing failure. |
| `retry_count` | default `0` | Persisted retry count. |

Indexes cover status, newest creation time, and non-null file hashes. Rust, not the database, enforces accepted `file_type` values and lifecycle transitions. Lifecycle values are `queued`, `embedding`, `indexed`, `deleting`, and `failed`. SQLite decides retrieval eligibility; LanceDB does not duplicate lifecycle state or provenance.

### `app_settings`

`app_settings` is a string key-value table. Its keys are application contracts, not SQL constraints.

| Key | Accepted value | Default when absent |
| --- | --- | --- |
| `theme` | `dark` or `light` | `dark` |
| `inference_mode` | `local` or `external` | `local` |
| `use_query_enhancement` | `true` or `false` | application-defined |
| `chunk_size` | Integer from 128 through 2048 | `512` |
| `chunk_overlap` | Integer from 0 through 1024 | `80` |
| `external_base_url` | Normalized credential-free HTTP or HTTPS base URL | local provider URL |
| `external_model` | Non-empty provider model identifier | `gpt-4o-mini` |
| `generation_model` | Valid Hugging Face model identifier | application-defined |
| `tokenizer_model` | Valid Hugging Face model identifier | `Qwen/Qwen3-1.7B` |
| `gguf_filename` | Non-empty `.gguf` filename without `/` | application-defined |

Chunk overlap must be smaller than chunk size. Changed settings requeue indexed documents with absent or mismatched provenance; old provenance remains until replacement succeeds.

The external-provider API key is stored in the OS keychain, with automatic fallback to unencrypted SQLite storage (`app_settings.external_api_key`) if keychain services are unavailable.

## LanceDB

The application-local `.lancedb` directory contains the `chunks` table. Each row is derived data for one document chunk.

| Field | Type | Meaning |
| --- | --- | --- |
| `vector` | fixed list of 768 `float32` values | Local embedding. |
| `document_id` | UTF-8 string | Logical reference to `documents.id`; it is not a database foreign key. |
| `page` | signed 32-bit integer | Physical PDF page or logical source page. |
| `chunk_index` | signed 32-bit integer | Chunk position within a source page. |
| `content` | UTF-8 string | Raw chunk content used for retrieval context and citations. |

Chunk identity is `(document_id, page, chunk_index)`. LanceDB retains raw content; embedding input adds `passage: `. Vector replacement precedes the SQLite transition to `indexed`.

## IPC commands

The frontend uses the typed IPC API.

| Command | Input | Result or outcome |
| --- | --- | --- |
| `begin_first_run_init` | none | Starts staged local initialization. |
| `retry_local_model_initialization` | none | Retries local generation-model initialization. |
| `cancel_model_download` | none | Aborts an active local model download and resets lifecycle state. |
| `get_app_state` | none | Memory-only probe returning `AppLifecycleState` with embedder, LLM, ingestion, and capacity snapshots. |
| `upload_document` | `filename`, `fileBytes`, `channel: Channel<IngestionProgress>` | Managed document identifier. |
| `list_documents` | none | Registry documents. |
| `get_document_preview` | `documentId` | Typed text or Markdown preview. |
| `delete_document` | `id` | Removes a managed document and derived data. |
| `execute_query` | `query`, `requestId`, `channel: Channel<TokenEvent>` | Streams answer events. |
| `cancel_query` | `requestId` | Cancels the named query request. |
| `get_settings` | none | String key-value settings. |
| `set_setting` | `key`, `value` | Persists one validated non-secret setting. |
| `test_external_provider` | `baseUrl`, optional `apiKey` | Available provider model identifiers. |
| `set_external_provider_config` | `baseUrl`, `model` | Persists external endpoint and model selection. |
| `set_api_key` | `key` | Stores the optional external-provider key in the keychain with unencrypted SQLite fallback. |
| `get_api_key_status` | none | Stored key existence and storage type (`keychain`, `database`, `none`). |
| `delete_api_key` | none | Whether a stored key was removed from keychain and database. |
| `get_storage_info` | none | Database, vector store, document, and model byte counts. |
| `compact_vector_store` | none | Completion status. |
| `vacuum_database` | none | Completion status. |
| `reset_all_data` | `deleteModels` | Whether restart was requested. |
| `get_platform` | none | Host operating system identifier: `macos`, `windows`, or `linux`. |
| `open_external_url` | `url` | Opens an external HTTP, HTTPS, or mailto link safely in the default system browser. |

Credential mutations report typed errors when cleanup is incomplete. A successful keychain save removes any plaintext fallback before reporting success. Deletion attempts both stores and returns `false` when neither contained a key; it never reports success if either removal fails. Data reset checks credential cleanup before writing the restart/reset marker.

`get_api_key_status` reports `database` whenever a plaintext fallback remains, including after a partial migration that also saved a keychain copy. This keeps the unencrypted-storage warning visible; inference still prefers the keychain key.

## Execution capacity and lifecycle state

`get_app_state` returns a memory-only snapshot of system readiness and execution permits without querying SQLite or LanceDB. Fields are read under separate locks rather than as one atomic snapshot; no fixed response latency is guaranteed:

```json
{
  "embedder_ready": true,
  "llm_ready": false,
  "inference_mode": "local",
  "model_download_active": true,
  "model_download_progress": {
    "model_id": "unsloth/Qwen3-1.7B-GGUF",
    "current_file": "Qwen3-1.7B-Q4_K_M.gguf",
    "file_downloaded": 104857600,
    "file_total": 1258291200,
    "files_completed": 0,
    "total_files": 1,
    "type": "downloading"
  },
  "active_queries_count": 0,
  "ingestion_active": false,
  "active_document_id": null,
  "capacity": {
    "ordinary_available": 16,
    "ordinary_capacity": 16,
    "protected_available": 4,
    "protected_capacity": 4
  }
}
```

Command execution capacity is strictly partitioned by priority via dual semaphores:
- **Ordinary capacity (16 permits):** Allocated to bulk commands (`upload_document`, `execute_query`, `delete_document`, `list_documents`, `get_document_preview`, settings, maintenance). Saturated capacity returns `ErrorCode::QueueFull`.
- **Protected capacity (4 permits):** Reserved strictly for interactive cancellations and probes (`cancel_query`, `cancel_model_download`, `get_app_state`). Background saturation or active inference loops cannot starve user cancellations or lifecycle polling.

## Request-scoped events

`TokenEvent` and `IngestionProgress` use snake-case `type` discriminators.

### `TokenEvent`

| Type | Fields | Meaning |
| --- | --- | --- |
| `token` | `text` | Answer text delta. |
| `warning` | `message` | Non-fatal retrieval degradation. |
| `done` | `citations` | Successful terminal event. |
| `error` | `message`, `citations` | Failure after answer streaming began. |

Failures before answer streaming reject `execute_query`. Citations have `document_id`, `filename`, `storage_path`, `page`, `chunk_content`, `score`, and `is_hybrid`. Citation `storage_path` is resolved for asset viewing; `documents.storage_path` is relative. `is_hybrid` marks a rank-derived, uncalibrated score.

### `IngestionProgress`

| Type | Fields |
| --- | --- |
| `extraction` | `document_id`, `current_page`, `total_pages` |
| `chunking` | `document_id`, `current_page`, `total_pages`, `chunks_on_page`, `total_chunks` |
| `embedding` | `document_id`, `current_batch`, `total_batches`, `embedded_chunks`, `total_chunks` |
| `stored` | `document_id`, `chunk_count` |
| `error` | `document_id`, `message` |

Progress reports completed work. `stored` follows vector replacement and registry update.

## Document preview

`get_document_preview` accepts an indexed text or DOCX document identifier and returns one tagged value:

| Kind | Fields |
| --- | --- |
| `plain_text` | `content`, `truncated` |
| `markdown` | `blocks`, `truncated` |

Markdown uses blocks `heading`, `paragraph`, `block_quote`, `list`, `code_block`, `table`, and `thematic_break`; inline values are `text`, `code`, `emphasis`, `strong`, `strikethrough`, `link`, and `break`. It is safe structure, not HTML. `truncated` signals the bounded source limit.

## Error payload

Command errors serialize as `{ code, operation, detail, recovery, message }`. `code` is a machine-readable snake_case identifier matching `ErrorCode`:

| Code | Meaning | Classification |
| --- | --- | --- |
| `not_found` | Requested resource, document, or key does not exist. | Terminal |
| `invalid_input` | Parameter validation, document schema, or extension error. | Terminal |
| `queue_full` | Ingestion pipeline capacity exhausted. | Transient |
| `duplicate_document` | Document content hash already indexed in registry. | Terminal |
| `storage` | SQLite, filesystem, or permission failure. | Transient |
| `vector_db` | LanceDB table creation, merge_insert, or index error. | Transient |
| `inference` | Local engine or external model execution failure. | Transient |
| `internal` | Unexpected system failure, panic, or unclassified defect. | Terminal |

- `operation`: The high-level action being performed when failure occurred (e.g. `Document upload`, `Query execution`).
- `detail`: The underlying technical cause or bounded diagnostic reason.
- `recovery`: Actionable, user-facing remediation instruction (e.g. `Wait a moment for existing documents to finish indexing, then retry.`).
- `message`: Formatted composite string (`"{operation}: {detail}. {recovery}"`) ensuring backward compatibility for consumers expecting a descriptive string message.
