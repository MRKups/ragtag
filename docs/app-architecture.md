---
title: Application architecture
normative-for: [system boundaries, document lifecycle, query lifecycle]
depends: [data-schema.md, decisions.md]
status: current reference. Source code and migrations override conflicting statements.
---

# Application architecture

Ragtag answers questions about user-provided documents. The interface runs in a Tauri WebView; Rust owns document processing, persistence, retrieval, and inference selection.

## System boundaries

| Subsystem | Responsibility | Authority boundary |
| --- | --- | --- |
| Svelte interface | Collects user input, renders results, and presents progress and recovery actions. | It uses the typed IPC boundary. |
| Tauri application | Validates requests, coordinates work, and exposes typed IPC commands. | It does not make the frontend a filesystem or database client. |
| Document pipeline | Extracts supported source formats, produces page-aware chunks, embeds text, and prepares previews. | It creates derived data, not canonical documents. |
| SQLite | Stores the document registry, lifecycle state, indexing provenance, and non-secret settings. | It decides whether a document is eligible for retrieval. |
| LanceDB | Stores derived chunks and vectors for local search. | It is recoverable from retained sources and the SQLite registry. |
| OS keychain | Stores the optional external-provider API key (falling back to SQLite if keychain is unavailable). | The key does not enter frontend storage, logs, or IPC payloads. |
| Inference providers | Generate an answer from a query and retrieved excerpts. | Local inference is the default. External inference is a user-selected trust boundary. |

[Data schema](data-schema.md) owns persisted fields, settings, and IPC vocabulary. [Design rationale](decisions.md) owns the rationale for durable choices.

## Data authority

Managed sources are retained locally before indexing; chunks, vectors, indexes, previews, and answers are derived.

SQLite is the document registry. A document becomes `indexed` after replacement chunks are durable. A `deleting` document is excluded until its derived data, managed source, and registry row are removed. Startup reconciles interrupted deletion and incomplete derived data.

## Document lifecycle

1. The interface submits a PDF, DOCX, or UTF-8 source through `upload_document` with a request-scoped progress channel.
2. The application validates the source, retains it under managed storage, and records it as queued.
3. Once the local embedder is available, the background pipeline extracts source text, preserves physical PDF page boundaries, and assigns one logical source page to DOCX and UTF-8 sources.
4. The pipeline creates tokenizer-boundary chunks, generates local embeddings, and replaces the document's derived rows in LanceDB.
5. After replacement succeeds, SQLite records indexed status and provenance. The originating request receives completed extraction, chunking, embedding, and storage progress.

Invalid input, a full work queue, source-integrity failure, and processing failure return typed errors. Failed work stays in the registry for recovery or retry.

## Query lifecycle

1. The interface submits a query and request identifier through `execute_query` with a request-scoped answer channel.
2. Ragtag embeds the query locally and searches only registry-indexed documents.
3. Hybrid full-text and vector retrieval supplies candidate chunks. If the hybrid path degrades, Ragtag continues with vector retrieval and sends a warning.
4. Ragtag removes unusable excerpts before inference. If none remain, the command rejects without generating an answer.
5. The selected provider streams answer events. Terminal events include citations from retrieved chunks, not model-generated page numbers.

`cancel_query` stops a pending or active inference request. It cannot interrupt already-running blocking embedding work.

## Inference lifecycle

The embedding model initializes locally at startup. Ingestion becomes available when it is ready. Query availability depends on the selected generation route.

Local mode downloads and loads the configured model in-process for answer generation.

External mode calls a user-selected OpenAI-compatible endpoint. Ragtag verifies the configured model before enabling queries and sends the query plus retrieved excerpts to that endpoint during inference. The endpoint URL is an SQLite setting; optional credentials are kept in the OS keychain (or SQLite as a fallback if the keychain is unavailable). Both HTTP (for local providers like Ollama or LM Studio) and HTTPS (for remote cloud providers) endpoints are supported. If no API key is specified, probing and inference proceed keyless; if provided, authentication headers are sent regardless of the endpoint scheme.

Default model files are downloaded on first use rather than bundled, using configurable Hugging Face repositories and GGUF filenames.

## User-visible failure behavior

Operations return serializable typed errors. Failures before answer streaming reject the command; later failures use a terminal error event with available citations. Ingestion progress reports completed work. The interface shows safe recovery guidance instead of raw backend details.
