---
title: Application architecture
normative-for: [system boundaries, document lifecycle, query lifecycle]
depends: [data-schema.md, decisions.md]
status: current reference. Source code and migrations override conflicting statements.
---

# Application architecture

Ragtag is a local-first desktop application for answering questions about user-provided documents. The interface runs in a Tauri WebView. Rust owns document processing, persistence, retrieval, and inference selection.

## System boundaries

| Subsystem | Responsibility | Authority boundary |
| --- | --- | --- |
| Svelte interface | Collects user input, renders results, and presents progress and recovery actions. | It calls typed IPC only through `frontend/src/lib/api.ts`. |
| Tauri application | Validates requests, coordinates work, and exposes typed IPC commands. | It does not make the frontend a filesystem or database client. |
| Document pipeline | Extracts supported source formats, produces page-aware chunks, embeds text, and prepares previews. | It creates derived data, not canonical documents. |
| SQLite | Stores the document registry, lifecycle state, indexing provenance, and non-secret settings. | It decides whether a document is eligible for retrieval. |
| LanceDB | Stores derived chunks and vectors for local search. | It is recoverable from retained sources and the SQLite registry. |
| OS keychain | Stores the optional external-provider API key. | The key does not enter SQLite, frontend storage, logs, or IPC payloads. |
| Inference providers | Generate an answer from a query and retrieved excerpts. | Local inference is the default. External inference is a user-selected trust boundary. |

[Data schema](data-schema.md) owns persisted fields, settings, and IPC wire vocabulary. [Design rationale](decisions.md) owns the reasons for durable choices.

## Data authority

Original user documents are managed sources. Ragtag retains a byte-for-byte application-local copy before indexing. Chunks, vectors, indexes, previews, and generated answers are derived data.

SQLite is the document registry. A document becomes `indexed` only after its replacement chunks are durable. A `deleting` document is excluded from retrieval until its derived data, managed source, and registry row are removed. Startup reconciles interrupted deletion and missing or incomplete derived data from the registry.

## Document lifecycle

1. The interface submits a PDF, DOCX, or UTF-8 source through `upload_document` with a request-scoped progress channel.
2. The application validates the source, retains it under managed storage, and records it as queued.
3. Once the local embedder is available, the background pipeline extracts source text, preserves physical PDF page boundaries, and assigns one logical source page to DOCX and UTF-8 sources.
4. The pipeline creates tokenizer-boundary chunks, generates local embeddings, and replaces the document's derived rows in LanceDB.
5. After replacement succeeds, SQLite records indexed status and provenance. The originating request receives completed extraction, chunking, embedding, and storage progress.

Invalid input, a full work queue, source-integrity failure, or processing failure produces a typed error. Failed work remains visible in the registry for recovery or retry. A bounded PDF source size does not yet bound decompression allocation before parser allocation. [B185](tracker.md) records that unresolved security limit.

## Query lifecycle

1. The interface submits a query and request identifier through `execute_query` with a request-scoped answer channel.
2. Ragtag embeds the query locally and searches only registry-indexed documents.
3. Hybrid full-text and vector retrieval supplies candidate chunks. If the hybrid path degrades, Ragtag continues with vector retrieval and sends a warning.
4. Ragtag removes unusable excerpts before inference. If none remain, the command rejects without generating an answer.
5. The selected provider streams answer events. Terminal events include citations from retrieved chunks, not model-generated page numbers.

`cancel_query` stops a pending or active inference request. It cannot interrupt already-running blocking embedding work.

## Inference lifecycle

The embedding model initializes locally at startup. Ingestion becomes available when it is ready. Query availability depends on the selected generation route.

Local mode downloads and loads the configured model in the application process. The model is retained in application state and serves local answer generation.

External mode calls a user-selected OpenAI-compatible endpoint. Ragtag verifies the configured model before enabling queries and sends the query plus retrieved excerpts to that endpoint during inference. The endpoint URL is a non-secret SQLite setting; the optional credential remains in the OS keychain. The current transport policy permits HTTP endpoints. [B186](tracker.md) records the required remote-transport restriction.

Default model files are downloaded rather than bundled. [B187](tracker.md) records the missing immutable revision and checksum verification for those downloads.

## User-visible failure behavior

Operations return serializable typed errors. Query failures before streaming reject the command. Failures after a response stream begins use a terminal error event with the retrieved citations available so far. Ingestion progress reports only completed work.

The interface presents safe recovery guidance rather than raw provider, filesystem, database, or authorization details.
