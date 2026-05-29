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

- [ ] 37. Remove `tauri-plugin-shell` from `src-tauri/Cargo.toml` dependencies.
- [ ] 38. Remove `shell:default` permission from `capabilities/default.json`.
- [ ] 39. Remove `.plugin(tauri_plugin_shell::init())` from `lib.rs` setup.
- [ ] 40. Remove `llm_token: Mutex<String>` from `AppState` (state.rs and lib.rs init).

### New Dependencies & State

- [ ] 41. Add `mistralrs = "=0.8.1"` to `src-tauri/Cargo.toml`.
- [ ] 42. Add `reqwest` (rustls-tls, stream, json), `keyring = "3"`, `hf-hub = "0.4"`, `rand = "0.9"` to `src-tauri/Cargo.toml`.
- [ ] 43. Add `Inference(String)` variant to `AppError` in `crates/domain/src/error.rs`.
- [ ] 44. Add `app_settings` get/set functions to `crates/persistence` (for `inference_mode`, `external_base_url`, `generation_model`, `isq_level`, etc.).
- [ ] 45. Initialize `mistralrs::Model` at startup via `TextModelBuilder` + `IsqType::Q4K`, store as `Arc<Model>` in `AppState`, emit `llm_ready` event.
- [ ] 46. Emit `embedder_ready` event after `fastembed` init completes.

### Vector Search Layer

- [ ] 47. Implement LanceDB Hybrid Search (FTS + dense vector via Reciprocal Rank Fusion) in `crates/rag_pipeline`.
- [ ] 48. Add vector-only fallback search if FTS index is missing or throws.
- [ ] 49. Implement query embedding via `fastembed` with `"query: "` prefix in `crates/rag_pipeline`.

### RAG Query Pipeline

- [ ] 50. Build RAG context prompt string from retrieved chunks in `crates/rag_pipeline`.
- [ ] 51. Implement `execute_query` Tauri command — accepts `query: String`, runs search → prompt → stream, returns via `Channel<TokenEvent>`.
- [ ] 52. Integrate `mistralrs` streaming: `model.stream_chat_request(messages)` → forward `Response::Chunk` deltas as `TokenEvent::Token`.
- [ ] 53. Attach structured citations (document_id, filename, page, chunk_content) to `TokenEvent::Done`.

### BYO Inference Mode

- [ ] 54. Integrate `keyring` for secure storage of `external_api_key` (service=`"ragtag"`, username=`"external_api_key"`).
- [ ] 55. Implement `reqwest`-based BYO inference path (POST to external OpenAI-compatible `/v1/chat/completions` with streaming).
- [ ] 56. Unify local `mistralrs` and external `reqwest` paths behind shared prompt/citation logic (trait or enum dispatch).
- [ ] 57. Wire up BYO routing: `execute_query` reads `inference_mode` from `app_settings`, routes accordingly.

## Phase 5: Polish, Security & UX

- [ ] 58. Integrate `hf-hub` for downloading `mistral.rs` model weights on first run with resume support.
- [ ] 59. Implement Staged Availability: unlock document ingest UI on `embedder_ready`, unlock query UI on `llm_ready` (skip LLM download if BYO mode active).
- [ ] 60. Build chat interface in Svelte (query textarea + submit button, streaming token display).
- [ ] 61. Build citation display component (show source page + filename when `TokenEvent::Done` fires).
- [ ] 62. Build settings UI (inference mode toggle, BYO endpoint URL, chunk size, ISQ level).
- [ ] 63. Integrate `pdf.js` for reliable cross-platform PDF rendering.
- [ ] 64. Emit progress events via `Channel<T>` for model downloads (hf-hub progress → frontend progress bar).
