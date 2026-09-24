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

The external-provider API key is stored only in the OS keychain.

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
| `has_api_key` | none | Whether an API key exists. |
| `get_api_key_status` | none | Stored key existence and storage type (`keychain`, `database`, `none`). |
| `delete_api_key` | none | Whether a stored key was removed from keychain and database. |
| `get_storage_info` | none | Database, vector store, document, and model byte counts. |
| `compact_vector_store` | none | Completion status. |
| `vacuum_database` | none | Completion status. |
| `reset_all_data` | `deleteModels` | Whether restart was requested. |
| `get_platform` | none | Host operating system identifier: `macos`, `windows`, or `linux`. |

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

Command errors serialize as `{ kind, message }`. Current `kind` values are `NotFound`, `Database`, `Io`, `VectorDb`, `QueueFull`, `Internal`, `Inference`, `DuplicateDocument`, and `InvalidInput`. Consumers branch on `kind`; messages are display text, not stable identifiers.
