---
title: File structure
normative-for: [repository navigation, package ownership, dependency direction]
depends: [app-architecture.md, data-schema.md, system-map.md]
status: current guide. Source paths override this navigation map.
---

# File structure

Use this document to locate a subsystem. It does not define runtime behavior. For granular capability-to-file mappings and data flow diagrams, refer to [System map](system-map.md).

## Root

- `README.md`: Source-only project introduction, app icon, screenshots, requirements, run/check commands, and privacy limitations.
- `assets/screenshots/`: Metadata-free WebP screenshots of query/citations, the document library, and dark/light settings for the README.
- `THIRD_PARTY_NOTICES.md` and `LICENSE`: First-party license and dependency notice summary.
- `scripts/check-release.mjs` and `scripts/check-release.test.mjs`: Version, repository identity, publishing, and inventory-freshness checks with regressions.
- `scripts/license-inventory.mjs`: Locked Rust/npm license-inventory generator.
- `Taskfile.yml`: Development, checking, build, bundle, and cleanup tasks.
- `docs/`: Current guides, specifications, and records. Start at [`docs/README.md`](README.md).
- `frontend/`: Svelte 5 and Vite interface.
- `src-tauri/`: Tauri application and Rust workspace.

## `docs/`

- `README.md`: Documentation index and reading order.
- `app-architecture.md`: Subsystem boundaries, authorities, and end-to-end lifecycle flows.
- `system-map.md`: Capability-to-subsystem mapping table, file inventory, and runtime data flows.
- `development.md`: Permanent, numbered development rules (`DEV-01` to `DEV-18`).
- `development-process.md`: How documentation is authored and maintained.
- `documentation-style.md`: Writing rules and the sentence test.
- `data-schema.md`: SQLite schemas, LanceDB record batches, settings, and IPC wire contracts.
- `decisions.md`: Durable architecture and technology decisions (`D001` to `D018`).
- `file-structure.md`: Repository layout and package boundaries.
- `glossary.md`: Repository-specific technical vocabulary.
- `tracker.md`: Active, unresolved implementation work items only.
- `open-source-readiness.md`: Public-release gate and packaging checklist.
- `third-party-license-inventory.json`: Generated third-party dependency license audit.

## `frontend/`

- `index.html`: Vite HTML entry point.
- `package.json` and `package-lock.json`: Frontend runtime and development dependencies.
- `src/main.ts`: Interface bootstrap and DOM mount point.
- `src/App.svelte`: Application desktop shell, top navigation bar, macOS window drag area, and global error container.
- `src/pages/`:
  - `QueryPage.svelte`: Question-and-answer view, query prompt, citation list, and document inspection drawer.
  - `DocumentsPage.svelte`: Document library inventory, drag-and-drop upload zone, and deletion dialogs.
  - `SettingsPage.svelte`: Model provider selection, RAG pipeline parameters, and storage maintenance controls.
- `src/lib/api.ts`: Sole frontend boundary allowed to import Tauri APIs (`@tauri-apps/api/core`).
- `src/lib/types.ts`: TypeScript contracts mirroring Rust IPC payloads and event types.
- `src/lib/citations.ts`: Shared citation-selection types and retrieval-to-viewer field mapping.
- `src/lib/router.svelte.ts`: Hash router with View Transitions API integration.
- `src/lib/error.ts`: Standardized error extraction and formatting.
- `src/lib/pdf-init.ts`: Local bundled PDF.js worker initializer.
- `src/lib/stores/`: Svelte 5 rune stores:
  - `app-state.svelte.ts`: Application readiness, model download state, and lifecycle events.
  - `query.svelte.ts`: Query execution, streaming tokens, citations, and cancellation.
  - `settings.svelte.ts`: Configuration state, provider selection, theme, and API key status.
  - `document-progress.svelte.ts`: Ingestion pipeline progress per document.
  - `error-dialog.svelte.ts`: Global modal error dialog state.
- `src/lib/components/`: Reusable interface components:
  - `AppInitializer.svelte`: Startup onboarding and model loading banner.
  - `AppSelect.svelte`: Accessible custom select dropdown.
  - `CitationList.svelte`: Retrieval-grounded citations with preview triggers.
  - `ConfidenceBadge.svelte`: Visual confidence rating (High / Medium / Low).
  - `DocumentList.svelte`: Document table with status badges and progress indicators.
  - `DocumentUpload.svelte`: Drag-and-drop upload area and file picker.
  - `DocumentViewer.svelte`: Drawer viewer coordinating PDF and Markdown previews.
  - `ErrorDialog.svelte`: Modal error alert with safe recovery actions.
  - `MarkdownPreview.svelte`: Safe rendered preview for text and DOCX documents.
  - `PdfPage.svelte`: Single-page PDF canvas renderer with text overlays.
  - `PdfViewer.svelte`: Virtualized PDF reader with zoom controls.
  - `QueryInput.svelte`: Auto-resizing query prompt input with submit/cancel keybindings.
  - `QueryResult.svelte`: Sanitized streaming answer renderer with markdown tokens and citations.
  - `SettingsTab.svelte`: Provider configuration, pipeline settings, and storage maintenance controls.
  - `Toggle.svelte`: Accessible switch toggle component.
- `src/lib/assets/`: Bundled frontend assets and icons.
- `src/lib/*.test.ts` and `src/lib/stores/*.test.ts`: Maintained frontend test suites.

## `src-tauri/`

- `Cargo.toml` and `Cargo.lock`: Rust workspace manifest and locked dependency resolution.
- `tauri.conf.json`: Tauri application and bundle configuration.
- `capabilities/`: Tauri security capability declarations.
- `icons/`: Native application icons.
- `src/main.rs`: Native binary entry point delegating to `ragtag_app::run()`.
- `examples/local_inference_smoke.rs`: Opt-in real-engine streaming/concurrency/receiver-drop checks over retained synthetic retrieval evidence; does not launch the personal application or qualify the WebView.
- `examples/local_generation_probe.rs`: Opt-in app-policy/eager/paged Metal comparison over synthetic input and an already-cached GGUF, with a diagnostic deadline and no application-document access.
- `src/lib.rs`: Application composition, builder setup, pending data resets, and background worker initialization; exports shared engine configuration/request helpers for real-model examples.
- `src/capacity.rs`: Dual-semaphore command capacity manager prioritizing interactive cancellations and probes over ordinary operations.
- `src/state.rs`: Central `AppState` holding pools, stores, embedders, capacity semaphores, active tracking, and worker channels.
- `src/paths.rs`: Shared Tauri application-data and model-directory resolution with typed errors.
- `src/credentials.rs`: Credential save/removal coordination, partial-failure reporting, and isolated adapter regressions.
- `src/model_download.rs`: Model download streamer supporting real-time progress emissions and Tokio watch cancellation.
- `src/commands/`: Focused Tauri IPC command modules:
  - `mod.rs`: Command module exports; `src/lib.rs` registers the handlers.
  - `documents.rs`: Document uploads, library queries, preview fetching, asset path resolution, and deletion.
  - `query.rs`: RAG query orchestration and query cancellation.
  - `models.rs`: Model status inspection, memory-only lifecycle state probe, local model downloads, download cancellation, and external provider probing.
  - `settings.rs`: Settings retrieval and updates.
  - `keys.rs`: Keychain and fallback API key operations.
  - `maintenance.rs`: Storage diagnostics, database compaction, factory reset coordination, and safe external URL dispatch.
- `src/inference/`: Inference execution:
  - `mod.rs`: `InferenceProvider` router multiplexing between local and external engines.
  - `local.rs`: In-process Mistral.rs execution streaming tokens with cancellation checks.
  - `external.rs`: OpenAI-compatible HTTP streaming provider with payload caps and keyless endpoint support.
  - `sse.rs`: Server-Sent Events parser handling chunk boundaries and multi-line events.
- `src/model_lifecycle/`: Model lifecycle management:
  - `mod.rs`: Module exports.
  - `coordinator.rs`: Generational state coordinator coalescing download, cancellation, and initialization requests.
  - `recovery.rs`: Shared vector-registry reconciliation and recovery of stale or interrupted indexing work.
- `src/workers/`: Background workers:
  - `mod.rs`: Module exports.
  - `ingestion.rs`: Dedicated worker processing document extraction, chunking, embedding, and indexing with backoff retries and active job tracking.
- `crates/`: Domain, persistence, and document-processing libraries.
- `.sqlx/`: Committed SQLx offline query metadata.
- `vendor/mistralrs-core/`: Licensed upstream v0.9.5 core with Ragtag's explicit tensor-selection repair ([D007](decisions.md)).
- `vendor/toml-selectors/`: Unchanged upstream test fixtures for the vendored core.

### `src-tauri/crates/domain/`

Adapter-free shared contracts using `serde` and `thiserror`. Defines `AppError`, `TokenEvent`, `IngestionProgress`, `ModelDownloadProgress`, `Document`, `Citation`, `DocumentStatus`, `IndexProvenance`, `ApiKeyStorage`, `AppLifecycleState`, `CapacityState`, `MAX_UPLOAD_BYTES`, and `EXTRACTED_TEXT_MAX_BYTES`.

### `src-tauri/crates/persistence/`

Relational, vector, and managed source storage:
- `sqlite.rs`: SQLite connection pool configuration, migrations in `crates/persistence/migrations/`, `DocumentRepository`, and `AppSettingsRepository`.
- `lancedb.rs`: Embedded `LanceDbVectorStore` with 768-d Float32 Arrow record batches and hybrid FTS search.
- `storage/`: Managed source storage, validation, and maintenance:
  - `mod.rs`: Shared application-directory validation and storage module exports.
  - `documents.rs`: Sandboxed source validation, magic-byte checking, atomic writes, and integrity hashing.
  - `reconciliation.rs`: Deletion finalization and orphan source file cleanup.
  - `maintenance.rs`: Disk footprint calculation and pending reset sentinel management.

### `src-tauri/crates/rag_pipeline/`

Document extraction, chunking, and embedding:
- `examples/retrieval_eval.rs`: Opt-in dense/hybrid/enhanced retrieval baseline over synthetic technical passages, using runtime embeddings and a new isolated output directory.
- `chunker.rs`: Token-budgeted text chunker preserving physical page boundaries.
- `lib.rs`: FastEmbed `TextEmbedder` for 768-d EmbeddingGemma with instruction prefixes.
- `extractor/`: Format-specific extractors:
  - `mod.rs`: Format detection and extraction router.
  - `pdf.rs`: PDF extractor preserving page boundaries.
  - `docx.rs`: Streaming XML extractor for DOCX files.
  - `text.rs`: Plain text extractor with UTF-8 validation and BOM removal.
  - `markdown.rs`: Markdown parser with HTML sanitization.
  - `util.rs`: Bounded source readers, preview truncation, and aggregate extraction limits.

## Dependency Direction

```text
frontend -> Tauri application -> domain, persistence, rag_pipeline
rag_pipeline -> domain, persistence
persistence -> domain
```

The Tauri application composes the libraries. `domain` remains free of adapter dependencies.
