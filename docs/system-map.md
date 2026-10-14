---
title: System and subsystem map
normative-for: [feature-to-subsystem mapping, file ownership, runtime data flows]
depends: [app-architecture.md, file-structure.md, data-schema.md]
status: current reference. Source code overrides conflicting statements.
---

# System and subsystem map

This document provides a comprehensive mapping of Ragtag's capabilities to their underlying software subsystems and the exact source files that implement them. Use it to understand ownership, data flows, and where specific behavior lives.

---

## 1. Master Capability Mapping Table

| Ragtag Feature / Capability | Subsystem in Code | Files that Compose that Subsystem |
|---|---|---|
| **Desktop Shell & Bootstrap** | Application Lifecycle & Composition | `frontend/src/main.ts`<br>`frontend/src/App.svelte`<br>`frontend/src/lib/components/AppInitializer.svelte`<br>`src-tauri/src/main.rs`<br>`src-tauri/src/lib.rs`<br>`src-tauri/src/state.rs` |
| **Typed Desktop Communication** | IPC Boundary & Data Contracts | `frontend/src/lib/api.ts`<br>`frontend/src/lib/types.ts`<br>`frontend/src/lib/error.ts`<br>`src-tauri/crates/domain/src/lib.rs`<br>`src-tauri/crates/domain/src/models.rs`<br>`src-tauri/crates/domain/src/error.rs`<br>`src-tauri/crates/domain/src/events.rs`<br>`src-tauri/src/commands/mod.rs` |
| **Client Routing & Navigation** | UI Navigation & Router | `frontend/src/lib/router.svelte.ts`<br>`frontend/src/App.svelte` |
| **Document Upload & Source Validation** | Ingestion Entry & Sandboxed Source Storage | `frontend/src/pages/DocumentsPage.svelte`<br>`frontend/src/lib/components/DocumentUpload.svelte`<br>`src-tauri/src/commands/documents.rs`<br>`src-tauri/crates/persistence/src/storage/documents.rs` |
| **Document Content Extraction** | Format-Specific Text Extraction | `src-tauri/crates/rag_pipeline/src/extractor/mod.rs`<br>`src-tauri/crates/rag_pipeline/src/extractor/pdf.rs`<br>`src-tauri/crates/rag_pipeline/src/extractor/docx.rs`<br>`src-tauri/crates/rag_pipeline/src/extractor/text.rs`<br>`src-tauri/crates/rag_pipeline/src/extractor/markdown.rs` |
| **Token Chunking & Instruction Prefixing** | Document Chunking Pipeline | `src-tauri/crates/rag_pipeline/src/chunker.rs` |
| **Text Embedding & Model Execution** | Embedding Engine | `src-tauri/crates/rag_pipeline/src/lib.rs` |
| **Document Ingestion Worker & Pipeline Coordination** | Background Ingestion Pipeline | `frontend/src/lib/stores/document-progress.svelte.ts`<br>`src-tauri/src/workers/ingestion.rs` |
| **Vector Storage, Indexing & Hybrid Search** | Vector & Lexical Persistence | `src-tauri/crates/persistence/src/lancedb.rs`<br>`src-tauri/src/model_lifecycle/recovery.rs` |
| **Document Registry & Metadata Management** | Relational Persistence (SQLite) | `src-tauri/crates/persistence/src/sqlite.rs`<br>`src-tauri/crates/domain/src/models.rs` |
| **Document Management & Inventory UI** | Document Management | `frontend/src/pages/DocumentsPage.svelte`<br>`frontend/src/lib/components/DocumentList.svelte`<br>`src-tauri/src/commands/documents.rs` |
| **Document Deletion & Garbage Collection** | Storage Reconciliation | `src-tauri/crates/persistence/src/storage/reconciliation.rs`<br>`src-tauri/src/commands/documents.rs` |
| **Document Inspection & Preview** | Document Viewer & Rendering | `frontend/src/lib/components/DocumentViewer.svelte`<br>`frontend/src/lib/components/PdfViewer.svelte`<br>`frontend/src/lib/components/PdfPage.svelte`<br>`frontend/src/lib/components/MarkdownPreview.svelte`<br>`frontend/src/lib/pdf-init.ts`<br>`src-tauri/crates/rag_pipeline/src/extractor/preview.rs`<br>`src-tauri/src/commands/documents.rs` |
| **RAG Query Execution & Coordination** | Query Orchestration | `frontend/src/pages/QueryPage.svelte`<br>`frontend/src/lib/stores/query.svelte.ts`<br>`frontend/src/lib/components/QueryInput.svelte`<br>`src-tauri/src/commands/query.rs` |
| **Local Model Inference** | Local In-Process Generation | `src-tauri/src/inference/local.rs`<br>`src-tauri/src/inference/mod.rs` |
| **External Model Inference** | Remote / OpenAI-Compatible Streaming | `src-tauri/src/inference/external.rs`<br>`src-tauri/src/inference/sse.rs`<br>`src-tauri/src/inference/mod.rs` |
| **Answer Streaming & Token Rendering** | Answer Presentation | `frontend/src/lib/components/QueryResult.svelte`<br>`frontend/src/lib/stores/query.svelte.ts` |
| **Grounding Citations & Relevance Display** | Grounded Citations | `frontend/src/lib/components/CitationList.svelte`<br>`frontend/src/lib/components/ConfidenceBadge.svelte`<br>`src-tauri/crates/domain/src/models.rs` |
| **Model Download & Readiness Coordination** | Model Lifecycle Coordination | `frontend/src/lib/stores/app-state.svelte.ts`<br>`src-tauri/src/model_lifecycle/coordinator.rs`<br>`src-tauri/src/model_download.rs`<br>`src-tauri/src/commands/models.rs` |
| **Application & Provider Settings Management** | Configuration & Preferences | `frontend/src/pages/SettingsPage.svelte`<br>`frontend/src/lib/stores/settings.svelte.ts`<br>`frontend/src/lib/components/SettingsTab.svelte`<br>`frontend/src/lib/components/Toggle.svelte`<br>`frontend/src/lib/components/AppSelect.svelte`<br>`src-tauri/src/commands/settings.rs`<br>`src-tauri/crates/persistence/src/sqlite.rs` |
| **Secure API Key Management** | OS Keychain & Fallback Security | `src-tauri/src/commands/keys.rs`<br>`src-tauri/src/credentials.rs`<br>`src-tauri/crates/persistence/src/sqlite.rs` |
| **Database Diagnostics & Maintenance** | Storage Maintenance | `src-tauri/src/commands/maintenance.rs`<br>`src-tauri/crates/persistence/src/storage/maintenance.rs` |
| **Application Data Factory Reset** | Disaster Recovery & Reset | `src-tauri/src/commands/maintenance.rs`<br>`src-tauri/crates/persistence/src/storage/maintenance.rs`<br>`src-tauri/src/lib.rs` |
| **User Error Reporting & Modal Feedback** | UI Error Presentation | `frontend/src/lib/stores/error-dialog.svelte.ts`<br>`frontend/src/lib/components/ErrorDialog.svelte` |

---

## 2. Granular Subsystem File Inventory

### 2.1 Frontend Interface (`frontend/src/`)

- **Root & Shell:**
  - `main.ts`: Application bootstrap, mounts `App.svelte` into DOM root.
  - `App.svelte`: Top-level desktop layout, macOS titlebar handling, navigation routing, global error listeners.
- **Contracts & Bridges:**
  - `lib/types.ts`: TypeScript interfaces for IPC payloads (documents, status, progress, settings, tokens, citations).
  - `lib/api.ts`: Typed IPC wrapper enforcing strict isolation around `@tauri-apps/api/core`.
  - `lib/error.ts`: Error formatting helper converting errors into human-readable text.
  - `lib/router.svelte.ts`: Hash-based SPA router with view transition integration.
  - `lib/pdf-init.ts`: Configures local bundled PDF.js worker.
- **Svelte 5 Rune Stores (`lib/stores/`):**
  - `app-state.svelte.ts`: Global readiness state, model download tracking, system event subscriptions.
  - `query.svelte.ts`: Active query streaming, token buffering, citation management, cancellation.
  - `settings.svelte.ts`: Provider configuration, endpoint URLs, API key status, theme mode.
  - `document-progress.svelte.ts`: Ingestion stage tracking and percentage mapping per document.
  - `error-dialog.svelte.ts`: Modal error presentation state.
- **Pages (`pages/`):**
  - `QueryPage.svelte`: Question-answering view, results display, citation drawer.
  - `DocumentsPage.svelte`: Document library inventory, drag-and-drop upload zone, delete actions.
  - `SettingsPage.svelte`: Provider selection, RAG tuning, database maintenance controls.
- **Reusable Components (`lib/components/`):**
  - `AppInitializer.svelte`: First-run onboarding and model loading banner.
  - `AppSelect.svelte`: Accessible styled dropdown component.
  - `CitationList.svelte`: Grounded citations list with page links.
  - `ConfidenceBadge.svelte`: Visual confidence rating (High / Medium / Low).
  - `DocumentList.svelte`: Paginated document table with status badges and progress indicators.
  - `DocumentUpload.svelte`: Drag-and-drop zone and native file picker trigger.
  - `DocumentViewer.svelte`: Drawer viewer coordinating PDF and Markdown previews.
  - `ErrorDialog.svelte`: Modal error alert with safe recovery actions.
  - `MarkdownPreview.svelte`: Safe rendered preview for text and DOCX documents.
  - `PdfPage.svelte`: Canvas rendering of single PDF pages with text selection overlays.
  - `PdfViewer.svelte`: Virtualized PDF document viewer with zoom controls.
  - `QueryInput.svelte`: Auto-resizing query prompt input with submit/cancel hotkeys.
  - `QueryResult.svelte`: Sanitized streaming answer renderer with markdown tokens and citations.
  - `SettingsTab.svelte`: Tab bar for settings navigation.
  - `Toggle.svelte`: Accessible binary toggle switch.

### 2.2 Core Backend Library Crates (`src-tauri/crates/`)

- **Domain Crate (`crates/domain/`):**
  - `Cargo.toml`: Zero-dependency domain definition (`serde`, `thiserror`).
  - `src/lib.rs`: Crate root re-exporting shared models and errors.
  - `src/error.rs`: Typed `AppError` enum and `{ kind, message }` serialization.
  - `src/events.rs`: Streaming channel event contracts (`TokenEvent`, `IngestionProgress`, `ModelDownloadProgress`).
  - `src/models.rs`: Primary entity structs (`Document`, `Citation`, `DocumentStatus`, `IndexProvenance`, `ApiKeyStorage`).
- **Persistence Crate (`crates/persistence/`):**
  - `Cargo.toml`: SQLite (`sqlx`), LanceDB (`lancedb`, `arrow`), and storage dependencies (`sha2`, `uuid`, `keyring`, `serde_json`).
  - `src/lib.rs`: Crate facade exposing repositories, vector store, and storage engine.
  - `src/sqlite.rs`: SQLite WAL pool, migration runner, document registry, settings repository, and fallback API key storage.
  - `src/lancedb.rs`: LanceDB embedded vector store, Arrow schema builder (768-d Float32), serialized `merge_insert` upserts, FTS index creation, and hybrid search.
  - `src/storage/`: Managed source storage, validation, and maintenance:
    - `mod.rs`: Module exports and facade re-exports.
    - `documents.rs`: Sandboxed managed file storage, path traversal protection, magic byte validation, and atomic writes.
    - `reconciliation.rs`: Orphan file cleanup and deletion finalization.
    - `maintenance.rs`: Disk footprint calculation and pending reset sentinel management.
- **RAG Pipeline Crate (`crates/rag_pipeline/`):**
  - `Cargo.toml`: Document extraction, tokenization, and FastEmbed dependencies.
  - `src/lib.rs`: `TextEmbedder` initializing EmbeddingGemma and computing embeddings with `passage: ` and `query: ` prefixes.
  - `src/chunker.rs`: Page-aware tokenizer chunking with token budgets, prefix reservations, and overlap windows.
  - `src/extractor/mod.rs`: Format detection and extraction router.
  - `src/extractor/pdf.rs`: PDF extractor preserving physical page provenance.
  - `src/extractor/docx.rs`: Streaming XML extractor for DOCX documents (`word/document.xml`) with memory limits.
  - `src/extractor/text.rs`: Plain text extractor validating UTF-8 and stripping BOMs.
  - `src/extractor/markdown.rs`: Markdown AST parser sanitizing HTML for safe preview rendering.
  - `src/extractor/preview.rs`: Document preview generator producing truncated plain text and structured blocks.

### 2.3 Tauri Application Shell (`src-tauri/src/`)

- **Composition & App State:**
  - `main.rs`: Native entry point delegating to `mrk_ragtag_app::run()`.
  - `lib.rs`: Bootstrap root, handles pending resets, opens databases, reconciles state, spawns ingestion worker, registers commands.
  - `state.rs`: Shared `AppState` struct holding connection pools, vector stores, channels, and model slots.
  - `model_download.rs`: Hugging Face download progress listener emitting channel updates.
- **IPC Command Handlers (`commands/`):**
  - `mod.rs`: Module exports and handler registration.
  - `documents.rs`: Document uploads, library queries, preview fetching, asset path resolution, and deletion triggers.
  - `query.rs`: RAG query orchestration and query stream cancellation.
  - `models.rs`: Model status inspection, local model downloads, download cancellation, and external provider probing.
  - `settings.rs`: Settings retrieval and updates.
  - `keys.rs`: Keychain and fallback API key operations.
  - `maintenance.rs`: Storage diagnostics, database compaction, and factory reset coordination.
- **Inference Execution (`inference/`):**
  - `mod.rs`: `InferenceProvider` router multiplexing between local and external engines.
  - `local.rs`: In-process Mistral.rs execution streaming tokens with cancellation checks.
  - `external.rs`: OpenAI-compatible HTTP streaming provider with payload caps and keyless endpoint support.
  - `sse.rs`: Robust Server-Sent Events parser handling chunk boundaries and multi-line events.
- **Model Lifecycle (`model_lifecycle/`):**
  - `mod.rs`: Module exports.
  - `coordinator.rs`: Generational state coordinator coalescing download and initialization requests.
  - `recovery.rs`: Startup database validator reconciling LanceDB tables and stale indexing records.
- **Background Workers (`workers/`):**
  - `mod.rs`: Module exports.
  - `ingestion.rs`: Background worker processing document extraction, chunking, embedding, and indexing with exponential backoff retries.

---

## 3. Runtime Lifecycle Workflows and Data Flows

### 3.1 Document Ingestion Flow

```text
User Selects File
       │
       ▼
frontend: DocumentUpload.svelte ──(IPC: upload_document)──► src-tauri: commands/documents.rs
                                                                     │
                                                                     ▼
                                                      storage/documents.rs (Atomic Sandbox)
                                                                     │
                                                                     ▼
                                                      persistence: sqlite.rs (Status: Queued)
                                                                     │
                                                                     ▼
                                                      workers/ingestion.rs (Background Queue)
                                                                     │
                                         ┌───────────────────────────┴───────────────────────────┐
                                         ▼                                                       ▼
                            rag_pipeline: extractor/                                rag_pipeline: chunker.rs
                           (PDF/DOCX/TXT extraction)                               (Token budget + overlap)
                                         │                                                       │
                                         └───────────────────────────┬───────────────────────────┘
                                                                     ▼
                                                        rag_pipeline: lib.rs (FastEmbed)
                                                          Prefix: "passage: "
                                                                     │
                                                                     ▼
                                                        persistence: lancedb.rs (merge_insert)
                                                                     │
                                                                     ▼
                                                        persistence: sqlite.rs (Status: Indexed)
                                                                     │
                                                                     ▼
                                                      Channel: IngestionProgress (Completed)
```

1. **Submission:** User drops a file in `DocumentUpload.svelte`. IPC command `upload_document` is invoked with a request-scoped progress channel.
2. **Validation & Storage:** `storage/documents.rs` validates file extension and magic bytes, sanitizes the filename, and writes the source file atomically into application-local managed storage.
3. **Registry Record:** SQLite creates a document record with status `queued`.
4. **Queue Dispatch:** The job is sent to `workers/ingestion.rs` via a bounded `mpsc` channel. If the queue is saturated, a typed queue-full error returns immediately.
5. **Extraction:** The worker invokes `rag_pipeline/src/extractor/` to stream text while preserving page provenance.
6. **Chunking:** `rag_pipeline/src/chunker.rs` breaks text into chunks within the 512-token limit, prepending the `passage: ` instruction prefix.
7. **Embedding:** `rag_pipeline/src/lib.rs` embeds chunks via FastEmbed EmbeddingGemma (768 dimensions).
8. **Vector Upsert:** `persistence/src/lancedb.rs` writes vectors using `merge_insert` on the composite key `(document_id, page, chunk_index)`.
9. **Finalization:** SQLite updates status to `indexed` with chunk count and provenance. The progress channel emits a terminal completion event.

### 3.2 Query Retrieval and Answer Streaming Flow

```text
User Submits Question
       │
       ▼
frontend: QueryPage.svelte ──────(IPC: execute_query)──────► src-tauri: commands/query.rs
                                                                     │
                                                                     ▼
                                                        rag_pipeline: lib.rs (FastEmbed)
                                                          Prefix: "query: "
                                                                     │
                                                                     ▼
                                                        persistence: lancedb.rs
                                                        (Hybrid Vector + FTS Search)
                                                                     │
                                                                     ▼
                                                        src-tauri: inference/mod.rs
                                                        (Router: Local vs External)
                                                                     │
                                         ┌───────────────────────────┴───────────────────────────┐
                                         ▼                                                       ▼
                            inference/local.rs (Mistral.rs)                         inference/external.rs (HTTP SSE)
                                         │                                                       │
                                         └───────────────────────────┬───────────────────────────┘
                                                                     ▼
                                                        Channel: TokenEvent::Token
                                                        (Streamed to QueryResult.svelte)
                                                                     │
                                                                     ▼
                                                        Channel: TokenEvent::Complete
                                                        (Grounded Citations from Retrieved Chunks)
```

1. **Submission:** User enters a question in `QueryInput.svelte`. IPC command `execute_query` passes a request-scoped channel and cancellation token.
2. **Embedding:** The query is formatted with the `query: ` prefix and embedded into a 768-dimensional vector.
3. **Retrieval:** `persistence/src/lancedb.rs` performs hybrid retrieval (vector similarity + BM25 full-text search) restricted exclusively to documents with status `indexed`. If hybrid search fails, it degrades to vector-only search and emits a `Degraded` event.
4. **Exclusion Check:** Unusable or low-scoring excerpts are filtered out. If no usable chunks remain, the query terminates without invoking the LLM.
5. **Prompt Assembly:** Grounded context chunks, system instructions, and the user prompt are assembled.
6. **Streaming Generation:**
   - **Local Mode:** `inference/local.rs` feeds the request to `mistralrs` and streams generated tokens across the channel.
   - **External Mode:** `inference/external.rs` issues a streaming HTTP POST request to the configured OpenAI-compatible endpoint, decoding tokens via `inference/sse.rs`.
7. **Citations:** Upon completion, the provider emits `TokenEvent::Complete` containing citations derived directly from retrieved chunk metadata (document ID, document title, physical page number).

### 3.3 Model Lifecycle Coordination Flow

1. **Readiness Probe:** Frontend boots and queries `get_model_status`.
2. **First-Run Onboarding:**
   - Embedder model is initialized first, unlocking document ingestion.
   - Local model download is initiated on demand or explicitly by the user.
3. **Coalesced Requests:** `model_lifecycle/coordinator.rs` maintains a generational state machine. If multiple requests trigger model downloads simultaneously, they share the single in-flight download future.
4. **Progress Forwarding:** Download progress is pushed over a dedicated channel to `app-state.svelte.ts`.
5. **Model Instantiation:** Once download completes, `mistralrs` initializes the model in-process (using Metal acceleration on Apple Silicon) and stores the instance in `AppState.local_model`.

---

## 4. Architectural Invariants and Safety Guarantees

1. **Original Documents as Evidence:** Uploaded PDFs and documents are saved byte-for-byte in managed storage. Vectors and chunks are derived data and never the canonical source.
2. **Retrieval-Grounded Citations:** Citation page numbers and titles come strictly from retrieved chunks, never from LLM token hallucinations.
3. **Strict IPC Boundary:** No Tauri API imports are permitted outside `frontend/src/lib/api.ts`.
4. **Domain Purity:** `crates/domain` has zero dependencies on databases, Tauri, Tokio, HTTP, or filesystem crates.
5. **Blocking Operation Isolation:** Blocking filesystem reads, PDF parsing, and model embeddings are always dispatched to `spawn_blocking` or dedicated worker threads.
6. **Crash-Safe Persistence:** Document chunk keys are deterministic `(document_id, page, chunk_index)`. Vector writes are serialized via `merge_insert`. Documents are marked `indexed` only after durable vector insertion.
7. **Secrets Isolation:** External API credentials are kept in the OS keychain (or unencrypted SQLite fallback only if keychain is unavailable), never exposed to frontend storage or logs.
