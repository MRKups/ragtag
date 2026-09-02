# File Structure

This document outlines the current layout of the application.

## Root Directory

- `package.json`: NPM workspace / scripts for driving both Svelte and Tauri builds.
- `Taskfile.yml`: Root development-task entry point. Defines normal Apple Silicon tasks plus `dev-intel` and `build-intel`, which validate and dynamically link the externally maintained Intel ONNX Runtime.
- `opencode.jsonc`: Agent tooling configuration.
- `docs/`: Architecture, specification documentation, and implementation tracker.
- `frontend/`: Plain Svelte 5 SPA (no SvelteKit) built with Vite.
- `src-tauri/`: The Tauri Rust application shell.

## `frontend/` (Plain Svelte 5 + Vite SPA)

The frontend is a plain Svelte 5 application compiled by Vite, running inside a Tauri WebView as a Single Page Application (SPA). Hash-based routing is used to avoid server-side routing complexities.

- `index.html`: Static HTML entry point with `#app` mount target.
- `package.json`: Plain Svelte deps — no `@sveltejs/kit` or adapter.
- `svelte.config.js`: Compiler options only (runes mode enforced, no `kit` config).
- `vite.config.ts`: Direct `svelte()` Vite plugin with manual `$lib` alias resolution.
- `tsconfig.json`: Standalone config (no `.svelte-kit/` extend), with `$lib/*` path mapping.
- `src/app.d.ts`: TypeScript declarations file (minimal — no SvelteKit `App` namespace).
- `src/main.ts`: Entry point — mounts `App.svelte` into `#app`.
- `src/App.svelte`: Root app shell with a compact, draggable macOS-overlay titlebar, header nav, `AppInitializer` boot gate, hash-based page routing via `{#key}` blocks, and CSS View Transitions.
- `src/pages/`:
  - `QueryPage.svelte`: Query interface with streaming chat (left) and a citation-source panel (right). It renders PDFs through PDF.js and Rust-owned typed previews for UTF-8 sources and DOCX: generic text is verbatim, while Markdown is rendered from safe block/inline data without HTML injection. Its keyed route lifecycle never cancels or resets the singleton query state, so active and completed answers survive hash-route navigation; a terminal answer clears only the composer.
  - `DocumentsPage.svelte`: Document management — upload (left) and list (right).
  - `SettingsPage.svelte`: Application configuration.
- `src/lib/`:
  - `index.ts`: Barrel re-export entry for the `$lib` path alias.
  - `api.ts`: Typed wrappers around Tauri `invoke` commands; the typed document-upload envelope owns its request-scoped `Channel<IngestionProgress>`, while remote-provider test/save calls expose model discovery and atomic configuration persistence.
  - `types.ts`: TypeScript interfaces mirroring Rust models, lifecycle events, provider-connection results, and the typed ingestion progress union.
  - `error.ts` and `error.test.ts`: Safe extraction of messages from typed IPC errors for task-specific UI recovery, with regression coverage for serialized `AppError` payloads.
  - `pdf-init.ts`: pdf.js worker initialization.
  - `router.svelte.ts`: Hash-based micro-router using Svelte 5 `$state` runes. Routes: `query`, `documents`, `settings`.
  - `assets/`: Static assets bundled by Vite (e.g., `favicon.svg`).
  - `stores/`: Svelte 5 `$state` stores:
    - `app-state.svelte.ts` — exports `AppState` and the `appState` singleton; tracks staged embedder/model readiness and model-load status.
    - `error-dialog.svelte.ts` and `error-dialog.test.ts` — export and exercise the route-independent queued `ErrorDialogState`; operations report safe notices with optional recovery actions, same-source deduplication, and lifecycle clearing.
    - `query.svelte.ts` — exports `QueryState` and the `queryState` singleton.
    - `settings.svelte.ts` — exports `SettingsState`, the `settingsState` singleton, and the shared local-inference defaults; also carries requested local/remote configuration dialogs across route navigation.
    - `document-progress.svelte.ts` — exports the route-independent `DocumentProgressState` singleton that immutably stores the latest request-scoped progress per document ID.
  - `components/`: Reusable UI components — `AppInitializer`, `AppSelect`, `ErrorDialog`, `CitationList`, `ConfidenceBadge`, `DocumentList`, `DocumentUpload`, `DocumentViewer`, `MarkdownPreview`, `PdfPage`, `PdfViewer`, `QueryInput`, `QueryResult`, `SettingsTab`, `Toggle`. `MarkdownPreview` maps the typed Rust preview model to semantic Svelte elements without an HTML sink. `AppSelect` provides the accessible, app-styled listbox control used for remote-provider protocol and discovered-model selection. `ErrorDialog` is the single accessible in-app recovery surface; `SettingsTab` presents local and remote inference cards as an accessible radio group, while either card opens an independent configuration modal.
- `static/`: Static frontend assets.

## `src-tauri/` (Tauri App Shell)

The Tauri adapter layer connecting the OS, the UI, and the Rust backend.

- `tauri.conf.json`: Core configuration, build instructions, asset-protocol scope, and the macOS overlay titlebar/traffic-light placement.
- `capabilities/`: JSON files defining exact capabilities mapped to specific window labels.
  - `default.json`: Grants the main window baseline permissions plus declarative titlebar dragging.
- `Cargo.toml`: Defines the Rust workspace, including the root Tauri v2 app and library crates in `crates/`. The root composes Tauri’s asset protocol, SQLx, Tokio, FastEmbed, ONNX Runtime, keyring, and inference dependencies; it does not register a general filesystem plugin.
- `.sqlx/`: Offline query cache generated by `cargo sqlx prepare`. Must be checked into version control so the CI and other developers can build without a live database.

### `src-tauri/src/`

The binary crate — entry point and thin command wrappers.

- `main.rs`: Barebones entry point calling `mrk_ragtag_app::run()`.
- `lib.rs`: Core setup and typed IPC commands. It initializes SQLite, `AppState`, and model/cache directories; initializes FastEmbed in `spawn_blocking`; serializes first Lance table creation; requires usable retrieved citations before inference; generation-tags local-model initialization and coalesces rapid local-model configuration changes so stale downloads, builds, progress, errors, and readiness cannot publish; validates indexed UTF-8 and DOCX preview requests against the document registry before reading their bounded source through the pipeline; tests draft OpenAI-compatible provider endpoints through the shared client, returns their discovered model IDs, atomically saves a chosen external endpoint/model before one revalidation, routes local/external inference through one Channel-event forwarder, and bridges channel-bearing document queue jobs to non-blocking ingestion-progress forwarders.
- `state.rs`: Defines `AppState` and `IngestionJob`; managed state holds the SQLite pool, document-work queue, late-initialized embedder, vector database, Arc-shared local MistralRS model, generation-tagged initialization state, and shared HTTP client.
- `model_download.rs`: `hf-hub` 1.0 model-download logic with resume support and a 3-attempt retry loop. Emits byte-level `model_download_progress` lifecycle events through a caller-supplied generation-safe progress handler.

### `src-tauri/crates/` (Business Logic Workspace)

Pure Rust library crates for portable, fast-testing business logic.

#### `crates/domain/` — Shared Types & Errors

Contains zero-infrastructure data types: models, errors, and IPC events. No `sqlx` or `tokio` dependencies.

- `Cargo.toml`: Minimal deps — `serde` and `thiserror`.
- `src/lib.rs`: Re-exports all public types.
- `src/models.rs`: `Document` struct (fields: id, filename, file_type, file_size_bytes, file_hash, storage_path, status, chunk_count, embedding_model, embedding_attempts, timestamps, successful-index provenance, error tracking), `IndexProvenance` (extraction version, chunk size, chunk overlap), and `DocumentStatus` enum (`Queued | Embedding | Indexed | Deleting | Failed`).
- `src/error.rs`: `AppError` enum with variants `NotFound`, `Database`, `Io`, `VectorDb`, `QueueFull`, `Internal`. Includes manual `Serialize` impl for Tauri IPC (structured `{ kind, message }`). `From<std::io::Error>` impl for `?` convenience.
- `src/events.rs`: `TokenEvent` enum (`Token | Warning | Done | Error`), `IngestionProgress` (`Extraction | Chunking | Embedding | Stored | Error`), and `Citation` — all derive `Serialize` + `Deserialize` for Tauri channels. `Citation::is_hybrid` prevents RRF rank from being presented as calibrated confidence.

#### `crates/persistence/` — Data Layer

Database and vector store operations. Depends on `domain`, `sqlx`, `lancedb`, `arrow-*`, `tokio`.

- `Cargo.toml`: Dependencies for SQLite (`sqlx 0.9`), LanceDB (`0.37.1`), and the matching Arrow `58.4` API family.
- `src/lib.rs`: Re-exports `DocumentRepository`, `VectorDb`, and `init_pool`. Contains a private `fn sql_err(e: sqlx::Error) -> AppError` helper to work around orphan rule restrictions on `From` impls.
- `src/sqlite.rs`:
  - `init_pool()`: Creates the app data directory, opens a SQLite connection with `SqliteConnectOptions`, runs migrations, returns a `SqlitePool` (max 5 connections).
  - `DocumentRepository`: CRUD operations — `insert`, status transitions, lookups, deletion, and recovery queries. Successful indexing stores typed `IndexProvenance` only after the vector upsert; stale-provenance requeue atomically changes only indexed rows with absent/mismatched provenance while preserving their prior values. `mark_deleting` persists a non-indexable deletion state; startup lists and completes those rows before normal document work. Date/time columns use `CAST(... as TEXT)` since SQLite has no native datetime type.
  - `AppSettingsRepository`: Key-value settings store — `get(key)` and `set(key, value)` with upsert on conflict. Its chunk-setting update transaction persists the final setting and requeues stale indexed rows before any producer enqueues them. Stores keys like `inference_mode`, `external_base_url`, `generation_model`, `chunk_size`, `chunk_overlap`, `use_query_enhancement`.
- `src/lancedb.rs`:
  - `VectorDb`: Holds a LanceDB `Connection`, a short first-table-creation mutex/fast path, and separate in-memory guards for FTS and IVF-PQ availability.
  - `init()`: Connects to a LanceDB directory on disk.
  - `schema()`: Arrow schema with `FixedSizeList(Float32, 768)` vectors, plus `document_id`, `page`, `chunk_index`, `content` columns.
  - `get_or_create_table()`: Serializes table discovery/creation only while `chunks` does not exist; subsequent calls open it concurrently. FTS index creation is deferred to the first write.
  - `upsert_chunks()`: Acquires a static `Semaphore(1)` write lock (prevents LanceDB commit conflicts), runs `merge_insert` on composite key `[document_id, page, chunk_index]` for idempotent upserts, creates FTS after rows exist, and defers 8-bit IVF-PQ until the table reaches 256 vectors.
  - `hybrid_search()`: Receives the SQLite snapshot of eligible indexed document IDs, applies its escaped filter before the FTS+dense hybrid/vector-only candidate limit, then returns deduplicated results capped at the requested limit. Falls back to vector-only if FTS is missing or errors, and does not expose RRF rank as calibrated confidence.
  - `delete_chunks()`: Deletes all chunk rows for a given `document_id` using a string filter expression. Used by durable document deletion.
- `migrations/`:
  - `20240101000000_initial.sql`: Creates `documents` table (id, file metadata, processing status, embedding metadata, timestamps, error tracking) with indexes on `status`, `file_hash` (unique), and `created_at`. Creates `app_settings` key-value table.
  - `20260901000000_document_index_provenance.sql`: Adds nullable `indexed_extraction_version`, `indexed_chunk_size`, and `indexed_chunk_overlap` columns for successful-index provenance.

#### `crates/rag_pipeline/` — Document Processing Pipeline

End-to-end document ingestion and source-preview logic. Depends on `domain`, `persistence`, `pdf-extract`, `zip`, `quick-xml`, `pulldown-cmark`, `fastembed`, `arrow-array`, `arrow-schema`, `tokio`.

- `Cargo.toml`: Dependencies for PDF extraction (`pdf-extract 0.12`), DOCX extraction (`zip 7.2`, `quick-xml 0.39`), typed Markdown parsing (`pulldown-cmark 0.13`), embeddings (`fastembed 6.0.2` backed by ONNX Runtime), tokenizer-boundary chunking (`tokenizers`), and the matching Arrow `58.4` API family.
- `src/lib.rs`: Re-exports `Chunk`. `init_embedder(cache_dir)` configures EmbeddingGemma with its 2,048-token limit. `process_document` receives typed `IndexProvenance`, a source file type, a lifecycle-stage callback, and a typed progress callback; it reports completed physical PDF pages or the one logical page for DOCX/UTF-8 sources, per-nonblank-page chunk totals, completed 32-item embedding batches, and durable storage only after the SQLite indexed/provenance update. It verifies complete extraction, prefixes only embedding inputs with `"passage: "`, writes vectors before SQLite `indexed` plus provenance, and propagates typed failures.
- `src/extractor.rs`: Dispatches physical-page PDF extraction, DOCX body-text extraction from `word/document.xml`, and UTF-8 source extraction. PDF preserves physical page numbers; DOCX and non-paginated text use logical page 1. Its bounded preview dispatcher returns generic plain text, DOCX-extracted text, or constrained typed Markdown; it caps output at 1 MiB, never splits a UTF-8 scalar, and drops raw HTML, images, embeds, and unsafe links.
- `src/chunker.rs`: Tokenizer-boundary chunker counts each temporary `"passage: "` embedding input with special tokens and keeps stored `Chunk { page, chunk_index, content }` raw for citations and prompts.
