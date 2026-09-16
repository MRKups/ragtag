---
title: Data schema and IPC contracts
normative-for: [persisted data, settings values, Tauri IPC wire vocabulary]
depends: [app-architecture.md, glossary.md]
status: current reference. Migrations and source code override conflicting statements.
---

# Data schema and IPC contracts

SQLite stores document identity and lifecycle state. LanceDB stores derived chunks and vectors. Tauri IPC carries requests and request-scoped progress. [Application architecture](app-architecture.md) owns the surrounding flows.

## SQLite

`app.db` is stored in application-local data. The migrations under `src-tauri/crates/persistence/migrations/` are the DDL source of truth. This document describes their effective schema.

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

Indexes cover document status, newest creation time, and non-null file hashes. The database does not enforce the accepted `file_type` values or lifecycle transitions. Rust enforces those application contracts.

Document lifecycle values are `queued`, `embedding`, `indexed`, `deleting`, and `failed`. SQLite is the authority for retrieval eligibility. LanceDB does not duplicate lifecycle state or indexing provenance.

### `app_settings`

`app_settings` is a `key` and `value` table. All values are strings. The accepted keys are application contracts, not SQL constraints.

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

The final chunk overlap must be smaller than the final chunk size. A changed chunk pair makes indexed documents with absent or mismatched provenance eligible for reindexing. The old provenance remains until replacement indexing succeeds.

The optional external-provider API key is not an application setting. It is stored in the OS keychain.

## LanceDB

The application-local `.lancedb` directory contains the `chunks` table. Each row is derived data for one document chunk.

| Field | Type | Meaning |
| --- | --- | --- |
| `vector` | fixed list of 768 `float32` values | Local embedding. |
| `document_id` | UTF-8 string | Logical reference to `documents.id`; it is not a database foreign key. |
| `page` | signed 32-bit integer | Physical PDF page or logical source page. |
| `chunk_index` | signed 32-bit integer | Chunk position within a source page. |
| `content` | UTF-8 string | Raw chunk content used for retrieval context and citations. |

The stable chunk identity is `(document_id, page, chunk_index)`. Embedding input adds the `passage: ` instruction prefix, but LanceDB retains raw chunk content. A successful vector replacement precedes the SQLite transition to `indexed`.

## IPC commands

The frontend may import Tauri APIs only in `frontend/src/lib/api.ts`. Components and stores call its typed functions.

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
| `set_api_key` | `key` | Stores the optional external-provider key in the keychain. |
| `has_api_key` | none | Whether that key exists. |
| `delete_api_key` | none | Whether a stored key was removed. |
| `get_storage_info` | none | Database, vector store, document, and model byte counts. |
| `compact_vector_store` | none | Completion status. |
| `vacuum_database` | none | Completion status. |
| `reset_all_data` | `deleteModels` | Whether restart was requested. |

## Request-scoped events

`TokenEvent` and `IngestionProgress` use a `type` discriminator with snake-case values.

### `TokenEvent`

| Type | Fields | Meaning |
| --- | --- | --- |
| `token` | `text` | Answer text delta. |
| `warning` | `message` | Non-fatal retrieval degradation. |
| `done` | `citations` | Successful terminal event. |
| `error` | `message`, `citations` | Failure after answer streaming began. |

Failures before answer streaming reject `execute_query`. Citations have `document_id`, `filename`, `storage_path`, `page`, `chunk_content`, `score`, and `is_hybrid`. Unlike the registry's relative `documents.storage_path`, citation `storage_path` is the resolved managed-source path used for asset viewing. `is_hybrid` means the score is rank-derived and not calibrated confidence.

### `IngestionProgress`

| Type | Fields |
| --- | --- |
| `extraction` | `document_id`, `current_page`, `total_pages` |
| `chunking` | `document_id`, `current_page`, `total_pages`, `chunks_on_page`, `total_chunks` |
| `embedding` | `document_id`, `current_batch`, `total_batches`, `embedded_chunks`, `total_chunks` |
| `stored` | `document_id`, `chunk_count` |
| `error` | `document_id`, `message` |

Progress describes completed work. `stored` follows durable vector replacement and registry update.

## Document preview

`get_document_preview` accepts an indexed text or DOCX document identifier. It returns one of these tagged values:

| Kind | Fields |
| --- | --- |
| `plain_text` | `content`, `truncated` |
| `markdown` | `blocks`, `truncated` |

Markdown blocks are `heading`, `paragraph`, `block_quote`, `list`, `code_block`, `table`, and `thematic_break`. Inline values are `text`, `code`, `emphasis`, `strong`, `strikethrough`, `link`, and `break`. The payload represents safe structure, not HTML. `truncated` signals that the preview reached its bounded source limit.

## Error payload

Command errors serialize as `{ kind, message }`. Current `kind` values are `NotFound`, `Database`, `Io`, `VectorDb`, `QueueFull`, `Internal`, `Inference`, `DuplicateDocument`, and `InvalidInput`. Consumers branch on `kind`; messages are display text, not stable identifiers.
