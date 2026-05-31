# Implementation Tracker

This document tracks the step-by-step implementation of the Ragtag architecture.

## Phase 1: Workspace & App Shell Foundation

- [x] 1. Initialize NPM `package.json` at the root for workspace management.
- [x] 2. Initialize Rust Cargo workspace in `src-tauri/Cargo.toml`.
- [x] 3. Create directory structure (`frontend/`, `src-tauri/crates/`, `docs/`).
- [x] 4. Scaffold SvelteKit app inside `frontend/`.
- [x] 5. Configure `svelte.config.js` for `adapter-static` (SPA mode with `fallback: 'index.html'`).
- [x] 6. Configure `vite.config.ts` (strict port `5173`, clear screen false).
- [x] 7. Use native Svelte 5 component-scoped CSS (no Tailwind).
- [x] 8. Update `tauri.conf.json` with build commands and basic identifiers.
- [x] 9. Configure IPC capabilities in Tauri (`core:allow-invoke` for testing).
- [x] 10. Configure `assetProtocol` in Tauri to allow reading from `$APPLOCALDATA/**` (required for PDF rendering).
- [x] 11. Write a dummy `ping` command in Rust (`#[tauri::command]`).
- [x] 12. Call `invoke('ping')` from Svelte on load and verify the message passes successfully.

## Phase 2: Data Layer & Persistence (The Brain)

- [x] 13. Define core data structures (`Document`, `DocumentStatus`, `AppError`, `TokenEvent`, `Citation`) in `crates/domain`.
- [x] 14. Implement `serde::Serialize` for `AppError` to ensure safe, typed errors over IPC.
- [x] 15. Add `sqlx` (v0.8.6) with `sqlite`, `runtime-tokio`, and `migrate` features to `crates/persistence`.
- [x] 16. Write SQL migrations for `documents` and `app_settings` tables.
- [x] 17. Implement DB initialization logic (ensuring the `app_local_data_dir` exists before connecting, running migrations).
- [x] 18. Implement CRUD repository functions for `documents` (`insert`, `update_status`, `get_by_id`, `reset_embedding_to_queued`).
- [x] 19. Add `lancedb` (v0.26.2) and `arrow` (57.2) crates to `crates/persistence`.
- [x] 20. Define the Arrow schema for chunks (`document_id`, `vector` [FixedSizeList 768], `page`, `chunk_index`, `content`).
- [x] 21. Implement LanceDB table creation and connection logic (`init`, `get_or_create_table`).
- [x] 22. Implement the LanceDB `merge_insert` pipeline with write `Semaphore(1)`, FTS index rebuild guard, and idempotent upserts on `[document_id, page, chunk_index]`.

## Phase 3: Background Processing Pipeline (The Muscle)

- [x] 23. Set up `tokio::sync::mpsc` channel (capacity 32) in Tauri's managed state within `lib.rs`.
- [x] 24. Implement the crash-recovery loop on startup (reset `embedding` docs to `queued` and re-queue stale `queued` docs).
- [x] 25. Spawn the main background worker task in `lib.rs` (runs full `rag_pipeline::process_document` and emits `document_indexed` / `document_failed` events).
- [x] 26. Create Tauri command `upload_document` — accepts `filename` + `file_bytes`, SHA-512 hash (in `spawn_blocking`), copies PDF to `$APPLOCALDATA/documents/`, inserts `queued` row, pushes doc ID via `try_send()`.
- [x] 27. Implement Rust logic to safely copy raw bytes to the App Data directory (via `tokio::fs`).
- [x] 28. Write initial SQLite `queued` row and push job to the MPSC channel.
- [x] 29. Integrate `pdf-extract` into `crates/rag_pipeline`.
- [x] 30. Write extraction loop (MUST be page-by-page to preserve citation boundaries).
- [x] 31. Implement token chunking logic (512 size / 80 overlap, `"passage: "` prefix).
- [x] 32. Integrate `fastembed-rs` (v5).
- [x] 33. Initialize `EmbeddingModel::EmbeddingGemma300M` inside a `spawn_blocking` thread on app startup.
- [x] 34. Hook embedding generation into the chunking pipeline.
- [x] 35. Enforce write order: Save vectors to LanceDB *first*, update SQLite to `indexed` *second*.
- [x] 36. Emit `document_indexed` Tauri event to Svelte upon completion.

## Phase 4: AI Execution & RAG (The Voice)

### Backtracking — Remove Sidecar Artifacts

- [x] 37. Remove `tauri-plugin-shell` from `src-tauri/Cargo.toml` dependencies.
- [x] 38. Remove `shell:default` permission from `capabilities/default.json`.
- [x] 39. Remove `.plugin(tauri_plugin_shell::init())` from `lib.rs` setup.
- [x] 40. Remove `llm_token: Mutex<String>` from `AppState` (state.rs and lib.rs init).

### New Dependencies & State

- [x] 41. Add `mistralrs = "=0.8.1"` to `src-tauri/Cargo.toml`.
- [x] 42. Add `reqwest` (rustls-tls, stream, json), `keyring = "3"`, `hf-hub = "0.4"`, `rand = "0.9"`, `lance-index = "2.0.0"` to `src-tauri/Cargo.toml`.
- [x] 43. Add `Inference(String)` variant to `AppError` in `crates/domain/src/error.rs`.
- [x] 44. Add `app_settings` get/set functions to `crates/persistence` (for `inference_mode`, `external_base_url`, `generation_model`, `isq_level`, etc.).
- [x] 45. Initialize `mistralrs::Model` at startup via `TextModelBuilder` + `IsqType::Q4K`, store as `Arc<Model>` in `AppState`, emit `llm_ready` event.
- [x] 46. Emit `embedder_ready` event after `fastembed` init completes.

### Vector Search Layer

- [x] 47. Implement LanceDB Hybrid Search (FTS + dense vector via Reciprocal Rank Fusion) in `crates/rag_pipeline`.
- [x] 48. Add vector-only fallback search if FTS index is missing or throws.
- [x] 49. Implement query embedding via `fastembed` with `"query: "` prefix in `crates/rag_pipeline`.

### RAG Query Pipeline

- [x] 50. Build RAG context prompt string from retrieved chunks in `crates/rag_pipeline`.
- [x] 51. Implement `execute_query` Tauri command — accepts `query: String`, runs search → prompt → stream, returns via `Channel<TokenEvent>`.
- [x] 52. Integrate `mistralrs` streaming: `model.stream_chat_request(messages)` → forward `Response::Chunk` deltas as `TokenEvent::Token`.
- [x] 53. Attach structured citations (document_id, filename, page, chunk_content) to `TokenEvent::Done`.

### BYO Inference Mode

- [ ] 54. Integrate `keyring` for secure storage of `external_api_key` (service=`"ragtag"`, username=`"external_api_key"`).
- [ ] 55. Implement `reqwest`-based BYO inference path (POST to external OpenAI-compatible `/v1/chat/completions` with streaming).
- [ ] 56. Unify local `mistralrs` and external `reqwest` paths behind shared prompt/citation logic (trait or enum dispatch).
- [ ] 57. Wire up BYO routing: `execute_query` reads `inference_mode` from `app_settings`, routes accordingly.

## Phase 5: Polish, Security & UX

- [x] 58. Integrate `hf-hub` for downloading `mistral.rs` model weights on first run with resume support.
- [ ] 59. Implement Staged Availability: unlock document ingest UI on `embedder_ready`, unlock query UI on `llm_ready` (skip LLM download if BYO mode active).
- [ ] 60. Emit progress events for model downloads (hf-hub progress → frontend progress bar).

## Phase 6: Frontend UI Components (The Face)

*Infrastructure ready: types.ts, api.ts, stores, pdf-init.ts, +layout shell, Toggle. UI components that consume them are below.*

- [x] 61. Build `SettingsTab.svelte` — full settings form: inference mode toggle, BYO fields (base URL + keyring-backed API key), local model repo, ISQ level, chunk size/overlap.
- [x] 62. Build `QueryInput.svelte` — chat-style textarea with Ctrl+Enter submit, character limit indicator, disabled state during streaming.
- [x] 63. Build `QueryResult.svelte` — streaming token display that binds to `queryState.text`, scrolls to bottom on new tokens, shows loading spinner during `isStreaming`.
- [x] 64. Build `CitationList.svelte` — list of citation cards showing filename, page number, and a preview of the chunk content. Clicking a citation sets `queryState.selectedDocument` to drive the PDF viewer.
- [x] 65. Build `ConfidenceBadge.svelte` — small pill component showing HIGH/MEDIUM/LOW confidence with color coding.
- [x] 66. Build `PdfViewer.svelte` + `PdfPage.svelte` — pdf.js rendered document viewer using `pdfjsLib` from `pdf-init.ts`. Renders visible pages in canvases with prev/next navigation and zoom. PdfPage handles per-page rendering with text layer and citation highlighting.
- [x] 67. Build `DocumentViewer.svelte` — wrapper component that hosts `PdfViewer` with a placeholder state. Converts `storagePath` to Tauri asset protocol URL via `convertFileSrc`.
- [x] 68. Build `+page.svelte` (root query page) — composite layout: `QueryInput` at top, `QueryResult` + `CitationList` in center, `DocumentViewer` in right panel. Orchestrates `api.executeQuery()` with channel callbacks feeding into `queryState`.

## Phase 7: Compile-Ready Polish & Bugfixes

Items surfaced during the 2026-05-29 build verification pass. The project compiles clean — `cargo check`, `cargo build`, `npm run build`, and `npm run check` all pass (0 TypeScript errors, 0 Rust errors). A 600MB `mrk-ragtag-app` binary builds successfully for arm64 macOS.

### Svelte 5 Warnings (20 total, 0 errors)

- [ ] 69. Fix deprecated svelte-ignore names in `QueryResult.svelte` — `a11y-no-static-element-interactions` → `a11y_no_static_element_interactions`, same for `a11y-click-events-have-key-events`.
- [ ] 70. Fix deprecated svelte-ignore names in `CitationList.svelte` — same rename as above.
- [ ] 71. Fix non-reactive DOM refs in `QueryResult.svelte` — `contentEl` should be `$state()`.
- [ ] 72. Fix non-reactive DOM refs in `PdfViewer.svelte` — `pdfDoc`, `scrollContainer` should be `$state()`.
- [ ] 73. Fix `state_referenced_locally` in `PdfViewer.svelte` — `pageNum` and `lastPageProp` capture initial `page` prop value rather than reacting to changes. Needs reactive prop pattern.
- [ ] 74. Add `aria-label` to PdfViewer toolbar buttons (prev/next/zoom-in/zoom-out).
- [ ] 75. Remove unused `.source-icon` CSS selector in `CitationList.svelte`.

### Rust Side Issues

- [ ] 76. Hardcoded model ID — `lib.rs:317` uses `mistralai/Mistral-7B-Instruct-v0.1` but AGENTS.md specifies Gemma 4 E2B. Read from `app_settings.generation_model` instead.
- [ ] 77. `embedder_ready` fires synchronously on line 358 before the async LLM init can complete. Both events should work with staged availability correctly (the frontend will gate on them).

### Frontend Cleanup

- [ ] 78. Remove unused `tailwind-merge` from `frontend/package.json` — no imports reference it.
- [ ] 79. Clean up unused aspirational types in `types.ts` — `QueryResponse`, `AnswerComponent`, `CitationData`, `ConfidenceLevel`, `Source` have no corresponding Rust structs and are never consumed.

---

## Phase 8: V1 Final Polish & Architecture Blueprint

### Stream A: GGUF Model Integration (Rust) — replaces hardcoded Mistral-7B

- [ ] 80. Swap `TextModelBuilder` → `GgufModelBuilder` in `lib.rs`. Replace with `GgufModelBuilder::new("unsloth/gemma-4-E2B-it-GGUF", vec!["gemma-4-E2B-it-Q8_0.gguf"])`. Remove `with_isq()` and `from_hf_cache_path()` (GGUF is pre-quantized, no HF cache path method).
- [ ] 81. Set `std::env::set_var("HF_HOME", &models_dir_llm)` before builder invocation so internal HF resolution finds the pre-downloaded single `.gguf` file.
- [ ] 82. Chain `.with_tok_model_id("google/gemma-4-E2B-it")` to fetch tokenizer from the safetensors repo (GGUF files lack tokenizer data).
- [ ] 83. Update `model_download.rs` — replace sibling-loop iteration with direct single-file: `repo.download_with_progress("gemma-4-E2B-it-Q8_0.gguf", reporter)`. Simplify to `total_files = 1`, remove the siblings loop, remove `emit_progress` helper.

### Stream B: Gemma Prompt Architecture (Rust)

- [ ] 84. Fix `build_rag_prompt` in `crates/rag_pipeline/src/lib.rs` — strip all system instructions. Output should only be: `[Document: X, Page: N]\n{chunk.content}\n\n` blocks followed by `User question: {query}`.
- [ ] 85. Fix `execute_query` in `lib.rs` — move all instructions to the System role message: `"You are a helpful assistant that answers questions using only the provided document excerpts. Answer concisely. Cite sources by indicating the page number in brackets like [Page N]. Do not use your own knowledge. Do not use <|think|> or </|think|> tags."`. The User message becomes the bare context string from `build_rag_prompt`.
- [ ] 86. Add `.enable_thinking(false)` to the `mistralrs::RequestBuilder` chain to suppress Gemma 4's `<|think|>` token.

### Stream C: Frontend Startup UX (Svelte) — covers tracker #59 and #60

- [ ] 87. Create `frontend/src/lib/stores/app-state.svelte.ts` — Svelte 5 `$state` class with `phase` (`'starting' | 'downloading_model' | 'ready' | 'error'`), `downloadProgress`, `errorMessage`. `init()` subscribes via `listen()` to `embedder_ready`, `model_download_progress`, and `llm_ready` events.
- [ ] 88. Create `frontend/src/lib/components/AppInitializer.svelte` — fullscreen dark overlay (`z-index: 9999`). Phase `'starting'` shows spinner + "Initializing Ragtag...". Phase `'downloading_model'` shows progress bar + filename. Phase `'error'` shows retry button. Phase `'ready'` renders empty (app visible underneath).
- [ ] 89. Gate `+layout.svelte` — wrap the `<div class="app-container">` in `{#if appState.phase === 'ready'}`, render `<AppInitializer />` in `{:else}`. Call `appState.init()` in `onMount`.

### Stream D: BYO External Inference (Rust + Frontend) — covers tracker #54, #55, #56, #57

- [ ] 90. Add `reqwest::Client` to `AppState` (`state.rs`). Build once in `setup()` with `rustls-tls`.
- [ ] 91. Add 3 keyring Tauri commands — `set_api_key` (uses `Entry::new("ragtag", "external_api_key").set_password()` in `spawn_blocking`), `has_api_key`, `delete_api_key`. All wrapped in `spawn_blocking`. Register in `invoke_handler`.
- [ ] 92. Implement BYO routing in `execute_query` — read `inference_mode` from `app_settings`. If `"external"`: read `external_base_url`, GET the API key from keyring, POST to `{base_url}/v1/chat/completions` with `{"stream":true}`, parse SSE line-by-line, extract `"choices"[0]["delta"]["content"]`, forward via `channel.send(TokenEvent::Token{...})`, attach citations on `[DONE]`.
- [ ] 93. BYO staged-availability bypass — in the LLM init background task, if `inference_mode == "external"`, skip download/build entirely and immediately `app.emit("llm_ready")`.
- [ ] 94. Wire frontend keyring functions — add `setApiKey`, `hasApiKey`, `deleteApiKey` to `api.ts`. Update `SettingsTab.svelte` API key field: replace the disabled placeholder with a real password input + "Save API Key" button + "Clear Key" button + success feedback.

### Stream E: Storage Management (Rust + Frontend) — new feature

- [ ] 95. Add `get_storage_info` command — returns byte counts for `app.db`, `.lancedb/`, `documents/`, and `models/` (excluding fastembed ONNX cache).
- [ ] 96. Add `compact_vector_store` command — calls `table.optimize()` or equivalent compact/cleanup on the LanceDB chunks table.
- [ ] 97. Add `vacuum_database` command — runs `VACUUM` on SQLite.
- [ ] 98. Add two-phase `reset_all_data(delete_models: bool)` command — writes `.reset_pending` sentinel file to `$APPLOCALDATA` with `{"delete_models": bool}`, returns `{ requires_restart: true }`. On next startup (before any pool/connection init): detect sentinel, purge `documents/`, `.lancedb/`, `app.db`, and conditionally `models/models--*` (preserving `models--fastembed`), remove sentinel, then proceed with normal init.
- [ ] 99. Build "Storage Management" UI section at the bottom of `SettingsTab.svelte` — displays metrics from `get_storage_info`, buttons for VACUUM, Compact, and Reset All (with confirmation dialog and "delete models?" checkbox). On reset confirm, show "Restart required" modal.

### Stream F: Cleanup & Polish

- [ ] 100. Remove unused `tailwind-merge` from `frontend/package.json` (tracker #78).
- [ ] 101. Remove unused `clsx` from `frontend/package.json`.
- [ ] 102. Clean unused types from `types.ts` — remove `QueryResponse`, `AnswerComponent`, `CitationData`, `ConfidenceLevel`, `Source` (tracker #79). Keep `AppError`.
- [ ] 103. Fix `DocumentUpload.svelte` `successCount` bug — add `successCount++` in the upload success branch so `uploadComplete` event fires.
- [ ] 104. Fix deprecated Svelte 5 ignore names — `a11y-click-events-have-key-events` → `a11y_click_events_have_key_events` in `CitationList.svelte` and `QueryResult.svelte` (tracker #69, #70).
- [ ] 105. Hide ISQ dropdown in `SettingsTab.svelte` when inference mode is `"local"` (GGUF is pre-quantized). Optionally show note: "Q8_0 quantization baked into the model file."
