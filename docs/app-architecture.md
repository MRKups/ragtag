---
title: Application architecture
normative-for: [system boundaries, document lifecycle, query lifecycle]
depends: [data-schema.md, decisions.md, system-map.md, development.md]
status: current reference. Source code and migrations override conflicting statements.
---

# Application architecture

Ragtag answers questions about user-provided documents. The interface runs in a Tauri WebView; Rust owns document processing, persistence, retrieval, and inference selection.

## System boundaries

| Subsystem | Responsibility | Authority boundary |
| --- | --- | --- |
| Svelte interface | Collects user input, renders streaming results, and presents progress and recovery actions. | Interacts exclusively through the typed IPC boundary in `src/lib/api.ts`. |
| Tauri application shell | Validates requests, coordinates lifecycle, and exposes typed IPC commands. | Composes native state and adapts IPC. Does not expose the filesystem or databases directly to the WebView. |
| Background ingestion worker | Consumes document ingestion jobs from a bounded queue with exponential backoff. | Pulls work asynchronously and emits progress events across request-scoped channels. |
| Document processing pipeline | Extracts supported source formats (PDF, DOCX, text, markdown), produces page-aware chunks, embeds text, and prepares previews. | Generates derived data, never rewriting canonical source documents. |
| Inference subsystem | Multiplexes between local in-process generation (`mistralrs`) and external OpenAI-compatible streaming endpoints with DoS safeguards. | Local inference is the default. External inference is a user-selected trust boundary. |
| Model lifecycle coordinator | Coalesces concurrent model download and initialization requests, and verifies vector store readiness. | Manages model state transitions and startup vector table recovery. |
| Storage & reconciliation | Sandboxes source uploads, validates magic bytes, and cleans up orphaned files and interrupted deletions. | Enforces filesystem boundaries and atomic writes. |
| SQLite | Stores the document registry, lifecycle state, indexing provenance, and non-secret settings. | Source of truth for document identity and retrieval eligibility. |
| LanceDB | Stores derived chunks, 768-d vectors, and FTS lexical indexes for local hybrid search. | Derived storage recoverable from retained sources and the SQLite registry. |
| OS keychain | Stores the optional external-provider API key (falling back to unencrypted SQLite only when keychain is unavailable). | API keys never enter frontend storage, logs, events, or diagnostics. |

[Data schema](data-schema.md) owns persisted fields, settings, and IPC vocabulary. [System map](system-map.md) owns granular capability-to-file mappings and data flow diagrams. [Design rationale](decisions.md) owns the rationale for durable choices. [Development rules](development.md) owns non-negotiable engineering rules.

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

On macOS, local generation explicitly uses non-paged Metal attention rather than the engine's paged default. Requests disable extended thinking through the engine's template control; the interface streams answer content, not hidden reasoning. [D007](decisions.md#d007--local-in-process-inference-by-default) records the runtime comparison and compatibility policy.

External mode calls a user-selected OpenAI-compatible endpoint. Ragtag verifies the configured model before enabling queries and sends the query plus retrieved excerpts to that endpoint during inference. The endpoint URL is an SQLite setting; optional credentials are kept in the OS keychain (or SQLite as a fallback if the keychain is unavailable). Both HTTP (for local providers like Ollama or LM Studio) and HTTPS (for remote cloud providers) endpoints are supported. If no API key is specified, probing and inference proceed keyless; if provided, authentication headers are sent regardless of the endpoint scheme.

Default model files are downloaded on first use rather than bundled, using configurable Hugging Face repositories and GGUF filenames.

Generation-model loading verifies GGUF bytes against the Hub cache's SHA-256 identity, including on cache hits. Verification stays local and responds to cancellation; incompatible or corrupt artifacts return typed errors rather than reaching the inference engine.

## User-visible failure behavior

Operations return serializable typed errors. Failures before answer streaming reject the command; later failures use a terminal error event with available citations. Ingestion progress reports completed work. The interface shows safe recovery guidance instead of raw backend details.
