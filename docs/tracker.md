# Implementation Tracker

This document tracks the step-by-step implementation of the Ragtag architecture.

## Phase 1: Workspace & App Shell Foundation

- [x] 1. Initialize NPM `package.json` at the root for workspace management.
- [x] 2. Initialize Rust Cargo workspace in `src-tauri/Cargo.toml`.
- [x] 3. Create directory structure (`frontend/`, `src-tauri/crates/`, `docs/`).
- [x] 4. Scaffold SvelteKit app inside `frontend/`.
- [x] 5. Configure `svelte.config.js` for `adapter-static` (SPA mode with `fallback: 'index.html'`).
- [x] 6. Configure `vite.config.ts` (strict port `5173`, clear screen false).
- [x] 7. Set up basic Tailwind CSS (or scoped Svelte CSS depending on final styling choice).
- [x] 8. Update `tauri.conf.json` with build commands and basic identifiers.
- [x] 9. Configure IPC capabilities in Tauri (`core:allow-invoke` for testing).
- [x] 10. Configure `assetProtocol` in Tauri to allow reading from `$APPLOCALDATA/**` (required for PDF rendering).
- [x] 11. Write a dummy `ping` command in Rust (`#[tauri::command]`).
- [x] 12. Call `invoke('ping')` from Svelte on load and verify the message passes successfully.

## Phase 2: Data Layer & Persistence (The Brain)

- [ ] 13. Define core data structures (`Document`, `AppState`, `AppError`) in `crates/domain`.
- [ ] 14. Implement `serde::Serialize` for `AppError` to ensure safe, typed errors over IPC.
- [ ] 15. Add `sqlx` (v0.8) with `sqlite`, `runtime-tokio`, and `migrate` features to `crates/persistence`.
- [ ] 16. Write SQL migrations for `documents` and `app_settings` tables.
- [ ] 17. Implement DB initialization logic (ensuring the `app_local_data_dir` exists before connecting).
- [ ] 18. Implement CRUD repository functions for `documents` (insert, update status).
- [ ] 19. Add `lancedb` (v0.26.2) and `arrow` crates to `crates/persistence`.
- [ ] 20. Define the Arrow schema for chunks (`id`, `vector` [FixedSizeList 768], `page`, `chunk_index`, `content`).
- [ ] 21. Implement LanceDB table creation and connection logic.
- [ ] 22. Implement the LanceDB `merge_insert` pipeline to guarantee idempotency on crashes.

## Phase 3: Background Processing Pipeline (The Muscle)

- [ ] 23. Set up `tokio::sync::mpsc` channel (capacity 32) in Tauri's managed state within `lib.rs`.
- [ ] 24. Implement the crash-recovery loop on startup (reset `embedding` docs to `queued` and re-queue).
- [ ] 25. Spawn the main background worker task in `lib.rs`.
- [ ] 26. Create Tauri command `upload_document`.
- [ ] 27. Implement Rust logic to safely copy raw bytes to the App Data directory.
- [ ] 28. Write initial SQLite `queued` row and push job to the MPSC channel.
- [ ] 29. Integrate `pdf-extract` into `crates/rag_pipeline`.
- [ ] 30. Write extraction loop (MUST be page-by-page to preserve citation boundaries).
- [ ] 31. Implement token chunking logic (512 size / 80 overlap).
- [ ] 32. Integrate `fastembed-rs` (v5).
- [ ] 33. Initialize `EmbeddingModel::EmbeddingGemma300M` inside a `spawn_blocking` thread on app startup.
- [ ] 34. Hook embedding generation into the chunking pipeline.
- [ ] 35. Enforce write order: Save vectors to LanceDB *first*, update SQLite to `indexed` *second*.
- [ ] 36. Emit `document_indexed` Tauri event to Svelte upon completion.

## Phase 4: AI Execution & RAG (The Voice)

- [ ] 37. Add `tauri-plugin-shell`, `command-group`, and `reqwest` to manage the sidecar.
- [ ] 38. Generate ephemeral auth token using `getrandom` on startup.
- [ ] 39. Implement dynamic ephemeral port binding for the LLM server.
- [ ] 40. Spawn `llama.cpp` sidecar process, verify health-check endpoint, and manage `RunEvent` cleanup hooks.
- [ ] 41. Implement LanceDB Hybrid search (Vector + BM25 via Reciprocal Rank Fusion).
- [ ] 42. Add fallback logic to vector-only search if FTS fails.
- [ ] 43. Build the RAG context prompt string from retrieved chunks in `crates/rag_pipeline`.
- [ ] 44. Implement communication with the OpenAI-compatible endpoint (local sidecar or external).
- [ ] 45. Implement Tauri v2 Channels (`Channel<TokenEvent>`) for streaming the response.
- [ ] 46. Ensure `TokenEvent::Done` includes structured metadata (page numbers) for the UI citation engine (no LLM hallucination).

## Phase 5: Polish, Security & UX

- [ ] 47. Integrate `keyring` crate.
- [ ] 48. Implement secure storage and retrieval of external API keys for BYO Inference mode.
- [ ] 49. Wire up bypass logic so the app routes to the external URL instead of the local sidecar when configured.
- [ ] 50. Integrate `hf-hub` for downloading model weights on first boot.
- [ ] 51. Implement "Staged Availability" (unlock UI when embedder is ready, even if LLM is downloading).
- [ ] 52. Build settings UI (managing models, BYO keys, chunk sizes).
- [ ] 53. Build chat interface.
- [ ] 54. Integrate `pdf.js` for reliable native-feeling PDF rendering across all platforms.