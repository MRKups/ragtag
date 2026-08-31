# Comprehensive Code Review Tracker — 2026-08-31

## Scope

A file-by-file audit for substantial correctness, reliability, security-boundary, lifecycle, and dead-code defects. Generated artifacts (`frontend/package-lock.json`, `src-tauri/Cargo.lock`, `src-tauri/target/`, and `src-tauri/gen/`) are excluded except where a source manifest or offline SQLx cache must agree with source. Findings are recorded only after a traced, evidence-backed investigation.

**Legend:** `[ ]` not reviewed; `[x]` reviewed, no substantial finding; `[F#]` contributes to a finding recorded below.

## Review Inventory

### Root, build, and runtime configuration

- [x] `package.json`
- [x] `README.md`
- [x] `.gitignore`
- [x] `rust-toolchain.toml`
- [x] `.nvmrc`
- [x] `frontend/index.html`
- [x] `frontend/.npmrc`
- [x] `src-tauri/build.rs`
- [x] `Taskfile.yml`
- [x] `frontend/vite.config.ts`
- [x] `frontend/svelte.config.js`
- [x] `frontend/tsconfig.json`
- [F7, F9] `src-tauri/Cargo.toml`
- [x] `src-tauri/tauri.conf.json`
- [F7] `src-tauri/capabilities/default.json`
- [x] `src-tauri/.sqlx/` query cache compared with SQLx macro calls
- [x] Rust workspace dependency and feature consistency

### Frontend entry, routing, and shared boundary

- [x] `frontend/src/main.ts`
- [x] `frontend/src/App.svelte` (confirmed dead `pages` map; listed below)
- [x] `frontend/src/app.d.ts`
- [x] `frontend/src/lib/pdf-init.ts`
- [x] `frontend/src/lib/api.ts`
- [x] `frontend/src/lib/types.ts`
- [x] `frontend/src/lib/router.svelte.ts`
- [x] `frontend/src/lib/index.ts` (absent; file-structure entry is stale)
- [x] `frontend/src/lib/assets/favicon.svg`

### Frontend stores and pages

- [x] `frontend/src/lib/stores/app-state.svelte.ts`
- [x] `frontend/src/lib/stores/query.svelte.ts`
- [x] `frontend/src/lib/stores/settings.svelte.ts`
- [x] `frontend/src/pages/QueryPage.svelte`
- [x] `frontend/src/pages/DocumentsPage.svelte`
- [x] `frontend/src/pages/SettingsPage.svelte`

### Frontend components

- [x] `frontend/src/lib/components/AppInitializer.svelte`
- [x] `frontend/src/lib/components/CitationList.svelte`
- [x] `frontend/src/lib/components/ConfidenceBadge.svelte`
- [x] `frontend/src/lib/components/DocumentList.svelte` (lower-severity listener/progress issues excluded)
- [x] `frontend/src/lib/components/DocumentUpload.svelte`
- [x] `frontend/src/lib/components/DocumentViewer.svelte`
- [x] `frontend/src/lib/components/PdfPage.svelte` (render-error spinner path excluded as lower-frequency)
- [F3] `frontend/src/lib/components/PdfViewer.svelte`
- [x] `frontend/src/lib/components/QueryInput.svelte`
- [F4] `frontend/src/lib/components/QueryResult.svelte`
- [F11] `frontend/src/lib/components/SettingsTab.svelte`
- [x] `frontend/src/lib/components/Toggle.svelte`

### Tauri adapter

- [x] `src-tauri/src/main.rs`
- [x] `src-tauri/src/state.rs`
- [x] `src-tauri/src/model_download.rs`
- [F4, F5, F6, F9] `src-tauri/src/lib.rs`

### Domain crate

- [x] `src-tauri/crates/domain/Cargo.toml` (unused `uuid` and `jiff` dependencies; listed below)
- [x] `src-tauri/crates/domain/src/lib.rs`
- [x] `src-tauri/crates/domain/src/error.rs`
- [x] `src-tauri/crates/domain/src/events.rs`
- [x] `src-tauri/crates/domain/src/models.rs`

### Persistence crate

- [x] `src-tauri/crates/persistence/Cargo.toml`
- [x] `src-tauri/crates/persistence/migrations/20240101000000_initial.sql`
- [x] `src-tauri/crates/persistence/src/lib.rs` (unconsumed `SearchResult` re-export; listed below)
- [x] `src-tauri/crates/persistence/src/sqlite.rs` (unconsumed `update_status`; listed below)
- [F8] `src-tauri/crates/persistence/src/lancedb.rs`

### RAG pipeline crate

- [x] `src-tauri/crates/rag_pipeline/Cargo.toml`
- [F2, F5] `src-tauri/crates/rag_pipeline/src/lib.rs`
- [F1] `src-tauri/crates/rag_pipeline/src/extractor.rs`
- [F2] `src-tauri/crates/rag_pipeline/src/chunker.rs`

### Documentation and historical tracker consistency

- [x] `docs/app-architecture.md`
- [x] `docs/data-schema.md`
- [x] `docs/decisions.md`
- [x] `docs/file-structure.md` (minor absent-`.env` and workspace wording inaccuracies excluded)
- [F10] `docs/tracker.md`
- [F10] `docs/review-2026-08-31.md`

## Confirmed Findings

### F1 — `pdf-extract` silently truncates documents at the first page-extraction failure

- **Affected code:** `src-tauri/crates/rag_pipeline/src/extractor.rs:4-8`; pinned `pdf-extract 0.12.0`, `src/lib.rs:2295-2306`.
- **Evidence:** The dependency loops with `while let Ok(...)` and returns its accumulated pages after *any* `extract_text_by_page` error. `extract_pages` accepts that partial vector as success and indexes it with no expected-page-count or partial-extraction check.
- **Impact:** A malformed or unsupported page after readable pages marks the document indexed with a silently truncated corpus and incomplete retrieval-grounded citations. Later pages are omitted instead of producing a retryable indexing failure.

### F2 — Configurable chunks exceed the embedding model’s hard token limit without detection

- **Affected code:** `src-tauri/crates/rag_pipeline/src/chunker.rs`; `src-tauri/crates/rag_pipeline/src/lib.rs:13-19, 49-64`; setting validation in `src-tauri/src/lib.rs`.
- **Evidence:** Ragtag accepts chunk sizes up to 2,048 estimated tokens and translates the maximum to an 8,192-character chunk. `fastembed 6.0.2` initializes `TextEmbedding` at its 512-token default and installs tokenizer truncation at that maximum. Full chunk content is kept in LanceDB and passed to the answer prompt, but only its first 512 tokens are embedded.
- **Impact:** Material portions of configured chunks are absent from similarity retrieval with no warning. The default 512-estimate chunk can also exceed the hard maximum for code, identifiers, or other text whose token-to-character ratio is lower than four.

### F3 — PDF next/previous controls cannot leave the selected citation page

- **Affected code:** `frontend/src/lib/components/PdfViewer.svelte:47-50, 92-100`.
- **Evidence:** A reactive effect continually copies the immutable `page` property into local `pageNum`. The toolbar changes `pageNum`, after which the effect resets it to the original citation page.
- **Impact:** Selecting a citation at page 10 then pressing Next produces page 11 only transiently before returning to page 10. Users cannot browse adjacent document pages.

### F4 — The answer body presents model-generated page citations as if they were grounded

- **Affected code:** `src-tauri/src/lib.rs:677-678`; `frontend/src/lib/components/QueryResult.svelte:22-73`.
- **Evidence:** The system prompt explicitly asks the model for `[Page N]` references. The documented contract says page numbers must come only from retrieved chunks. `QueryResult` renders model Markdown as text and has no mapping from those references to structured citations; its click handler expects absent `data-citation-id` attributes. The separate citation list remains grounded but cannot validate text inside the answer.
- **Impact:** Answers can display invented or ambiguous page references, directly undermining the product’s retrieval-grounded citation guarantee.

### F5 — Empty usable retrieval context still invokes the model and can return an uncited answer

- **Affected code:** `src-tauri/src/lib.rs:653-679, 686-879`; `src-tauri/crates/rag_pipeline/src/lib.rs:194-209`.
- **Evidence:** After registry lifecycle filtering, the citation list can be empty. `build_rag_prompt` then produces only `User question: …`; the command still invokes either inference backend and sends `Done` with no citations. There is no no-context refusal or user-visible retrieval failure.
- **Impact:** Queries before any successful ingest, or during stale-heavy vector state, can yield answers from model knowledge without retrieval evidence, violating the local RAG product contract.

### F6 — Switching to external inference retains and can later reuse an obsolete local model

- **Affected code:** `src-tauri/src/lib.rs:355-365, 1070-1080`.
- **Evidence:** Setting `inference_mode=external` advances initialization generation and emits readiness but never clears `AppState.local_llm`. Returning to local calls `begin_first_run_init`, which treats any existing model as ready and skips initialization.
- **Impact:** External mode retains a large, unnecessary in-process model. More seriously, changes to local model settings while external is selected are ignored on return to local mode: the old model serves requests despite the persisted configuration.

### F7 — The renderer has unnecessary recursive filesystem capability over all app data

- **Affected code:** `src-tauri/capabilities/default.json:11`; `src-tauri/Cargo.toml:21`; plugin registration in `src-tauri/src/lib.rs:1312-1313`.
- **Evidence:** `fs:default` grants recursive read access to AppConfig, AppData, AppLocalData, AppCache, and AppLog through the registered filesystem plugin. No frontend code imports or invokes the filesystem plugin; PDF display uses the core asset protocol.
- **Impact:** A compromised renderer can read the SQLite registry, derived vector data, stored user PDFs, and model cache without a product need. This violates the intended minimal Tauri IPC/capability boundary.

### F8 — First access to the LanceDB table has an unsynchronized create race

- **Affected code:** `src-tauri/crates/persistence/src/lancedb.rs:54-80, 114-128, 224-230`; query path in `src-tauri/src/lib.rs:633-638`.
- **Evidence:** `get_or_create_table` performs `table_names` and `create_empty_table` without synchronization. `upsert_chunks` resolves the table before acquiring `VECTOR_WRITE_LOCK`, and `hybrid_search` resolves it without the lock. A first upload plus immediate query can both observe no table; one non-idempotent create then fails with `AlreadyExists`.
- **Impact:** An otherwise valid first ingestion is marked failed and remains unavailable until recovery/retry. The existing write semaphore does not cover the race.

### F9 — “Delete downloaded models” does not delete tokenizer files

- **Affected code:** `src-tauri/src/lib.rs:240-242, 307-311, 1269-1272`; `frontend/src/lib/components/SettingsTab.svelte:496-500`.
- **Evidence:** GGUF weights use the application-local model directory, but `mistralrs::GgufModelBuilder::with_tok_model_id` resolves tokenizer assets through the default Hugging Face cache. Reset deletes only application-local models while the UI promises to delete downloaded models and accounts only for that local directory.
- **Impact:** Reset leaves model artifacts behind, consumes unreported storage, and violates the user-visible data-cleanup promise.

### F10 — Active implementation tracker contradicts completed remediation record

- **Affected code:** `docs/tracker.md:3, 14-19`; `docs/review-2026-08-31.md:110-125`.
- **Evidence:** The review records the relevant remediation as complete, while the active tracker still lists B112, B121, B132, B133, B134, and B137 as unresolved work.
- **Impact:** Maintainers receive contradictory source-of-truth guidance and can duplicate, revert, or mis-prioritize reliability work.

### F11 — Rapid inference-mode changes can persist an earlier selection

- **Affected code:** `frontend/src/lib/components/SettingsTab.svelte:39-50, 68-72`.
- **Evidence:** Updates are serialized only after `setDefaultInference` decides whether to enqueue them. That decision compares `mode` against stale persisted state. Selecting external and then local before the first IPC write resolves makes the second call return without enqueuing local; the first write persists external.
- **Impact:** A user who changes their mind can be left on the external inference route despite selecting local. This is particularly consequential because it changes whether queries leave the default local-inference path.

## Confirmed Dead Code and Low-Impact Removals

- `frontend/src/App.svelte:12-16`: `pages` is constructed but never read; route rendering imports components directly.
- `frontend/src/lib/stores/app-state.svelte.ts:142-145`: `destroy()` has no caller; the root app never invokes the retained listener cleanup path.
- `frontend/src/lib/components/PdfViewer.svelte:27, 143`: `scrollContainer` is only bound, never read.
- `src-tauri/crates/domain/Cargo.toml`: `uuid` and `jiff` have no domain-crate callsites.
- `src-tauri/crates/persistence/src/sqlite.rs:85-103`: `DocumentRepository::update_status` has no workspace callsite.
- `src-tauri/crates/persistence/src/lib.rs:4`: the root `SearchResult` re-export has no workspace consumer. Keep it only if it is intentional public library API.

## Investigated and Excluded

- The Lance schema lacks an explicit migration mechanism, but the initial repository version already used the current `vector`, `document_id`, `page`, `chunk_index`, and `content` schema. No incompatible released schema was established, so this is not recorded as a current defect.
- Lance compaction bypasses Ragtag’s application lock, but LanceDB’s documented concurrent optimize/add support means a failure sequence was not established.
- DocumentList listener cleanup timing, live progress reactivity, query auto-scroll, and PdfPage’s render-error spinner have bounded or UI-only impact; they do not meet this review’s substantial-issue threshold.
- `docs/file-structure.md` refers to an absent `frontend/src/lib/index.ts`; this is a harmless navigation-documentation discrepancy.

## Verification Log
- `npm run check` passed: 4,031 files checked, 0 errors, 0 warnings.
- `npm run build` passed. Vite reports the existing 619 kB minified entry chunk warning.
- `cargo test --workspace` passed with the documented Intel external ONNX Runtime environment: 12 tests across 9 suites.
- The source-level findings above were traced through callers, dependency source where relevant, current configuration, and the initial Lance schema revision.

## Remediation — 2026-08-31

- **F1 resolved:** page-count verification rejects partial `pdf-extract` output before blank-page filtering.
- **F2 resolved:** ingest chunks at active tokenizer boundaries, counts the embedding-only `passage: ` prefix and special tokens, and bumps `extraction_version` to `token-aware-v2` so existing indexed documents requeue. Stored citation/prompt content remains raw.
- **F3 resolved:** citation-page updates synchronize the viewer once; manual page navigation no longer resets.
- **F4 resolved:** inference is prohibited from emitting page/source markers, and obsolete answer-body citation-click plumbing was removed.
- **F5 resolved:** inference is not called unless at least one indexed, non-empty retrieved citation remains after lifecycle filtering.
- **F6 resolved:** inference-mode changes invalidate pending local initialization; external mode releases the local model and returning local rebuilds current settings.
- **F7 resolved:** the unused filesystem plugin and `fs:default` capability were removed; PDF access remains through Tauri's asset protocol.
- **F8 resolved:** first `chunks` table creation is serialized without serializing normal reads after table existence is known.
- **F9 resolved by scope correction:** reset now accurately promises deletion of app-managed local model downloads. A Hugging Face cache explicitly selected through `HF_HUB_CACHE` or `HF_HOME` remains user-managed, unreported, and untouched.
- **F10 resolved:** the historical remediation report now distinguishes completed reviewed findings from still-active tracker follow-ups.
- **F11 resolved:** inference-mode selections always enqueue through the existing per-key serializer, preserving the latest selection.
- **Dead code removed:** obsolete route/page and PDF bindings, root-store cleanup method, domain dependencies, persistence method, re-export, and answer-body citation handler.

- Remediation verification: `cargo test -p rag_pipeline` passed 7 tests; `cargo fmt --all && cargo test --workspace` passed 17 tests across 9 suites with the documented Intel external ONNX Runtime environment.
- Remediation verification: `npm run check && npm run build` passed with 0 Svelte diagnostics. Vite retains its existing 618 kB minified-entry warning.
- Remediation verification: `cargo tauri build --no-bundle` completed successfully and built `target/release/mrk-ragtag-app`.
