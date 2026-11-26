---
title: Design rationale
normative-for: [durable architecture and technology decisions]
depends: [app-architecture.md, data-schema.md]
status: current decision record. Source code and migrations implement the accepted decisions.
---

# Design rationale

This document records durable choices and tradeoffs. Linked references own current behavior. Records are accepted unless superseded.

## D001 — Tauri desktop application

**Context:** Ragtag must run as a local desktop application without a required hosted service.

**Decision:** Use Tauri with a Svelte interface and a Rust application layer.

**Consequences:** The application ships as a native desktop bundle. The WebView uses typed IPC for native work. Local operating-system capabilities remain explicit Tauri concerns.

## D002 — Tauri IPC boundary

**Context:** The interface needs access to document, retrieval, and settings operations without becoming a native-storage client.

**Decision:** Use Tauri v2 commands and request-scoped channels for frontend-to-Rust communication.

**Consequences:** The frontend stays behind the typed IPC boundary; [Data schema and IPC contracts](data-schema.md) defines its wire vocabulary.

## D003 — SQLite document registry

**Context:** Document identity, lifecycle state, and settings must survive restarts and coordinate recoverable derived data.

**Decision:** Use embedded SQLite as the registry and settings store.

**Consequences:** SQLite provides restart-safe authority for retrieval eligibility and indexing provenance; see [Data schema and IPC contracts](data-schema.md).

## D004 — In-memory document work queue

**Context:** Document processing is asynchronous but depends on durable registry state rather than an external work service.

**Decision:** Use an in-process bounded queue for document work, with startup recovery from SQLite state.

**Consequences:** The queue improves interactive responsiveness but is not the source of truth. Queued and interrupted work remains recoverable from the registry.

## D005 — Application-local managed sources

**Context:** User documents are evidence for citations and must remain distinct from generated output and indexes.

**Decision:** Retain managed copies in application-local storage and treat them as canonical evidence.

**Consequences:** Managed sources make derived indexing and generated output rebuildable.

## D006 — Embedded LanceDB retrieval

**Context:** Ragtag needs local vector and full-text retrieval without a separate database service.

**Decision:** Use embedded LanceDB for derived chunks and vectors.

**Consequences:** LanceDB is recoverable derived storage and does not own lifecycle state; [Application architecture](app-architecture.md) defines retrieval degradation.

## D007 — Local in-process inference by default

**Context:** Default answers and embeddings must remain local while allowing an explicit provider choice for users who need external inference.

**Decision:** Run FastEmbed embeddings and `mistral.rs` generation in the application process by default. Support user-selected OpenAI-compatible external inference as an exception.

**Consequences:** Default processing, retrieval, and generation need no model server; external inference crosses the configured provider boundary. Credentials reside in the OS keychain with automatic fallback to SQLite when unavailable; external endpoints support standard HTTP for local servers (Ollama, LM Studio) and HTTPS for cloud providers.

**Dependency source:** Pin `mistral.rs` v0.9.5 to release-tag Git revision `465191d0b0a7e2432d327b0de06204ab3f94d3a2` with Metal acceleration on Apple Silicon. This is the latest stable upstream release checked on 2026-11-26. Retain the resolved `Cargo.lock`, including upstream's Candle 0.11.0 revision `66a8cf184a5a519671454066b1b9efd446ec9f5c`.

**Maintained deviation:** Override only `mistralrs-core` with the licensed source in `src-tauri/vendor/mistralrs-core`. Patch `ragtag.1` makes narrowed batched logits selections contiguous before quantized projection; it preserves concurrent inference rather than adding a request semaphore. The vendored manifest resolves upstream workspace inheritance and keeps sibling packages on the same Git revision. Its build script reports the upstream revision plus `ragtag.1`, not the enclosing application's Git HEAD. All other upstream source remains unchanged.

**Maintenance boundary:** Keep the imported package and license intact, with the manifest normalization, revision reporting, and selection repair as explicit deviations. Preserve the two original fixtures in the adjacent `vendor/toml-selectors/`; the upstream crate needs them even when compiling a filtered test target. Its focused regression is part of the native test tasks; the rest of the upstream suite is not implicitly part of Ragtag's workspace. Retire the override only after an upstream replacement provides the repair and equivalent regression and real-model qualification pass.

**Qualification:** On 2026-11-26, the v0.9.5 stack compiled with Metal on Rust 1.100.0 and passed 168 workspace tests plus the batched quantized-projection regression. The local patch remains necessary: upstream still narrows batched logits without making them contiguous. The macOS test linker emitted a large `__eh_frame` / compact-unwind warning; no warning was suppressed. The prior v0.9.4 baseline passed real Metal streaming, concurrent queries, receiver-drop recovery, and cached offline initialization. Requalify these scenarios after v0.9.5's Metal paged-attention and memory-management changes. [B189](tracker.md) tracks current development-app acceptance; this repository does not distribute installers.

**macOS execution policy:** The first real desktop query stalled after retrieval, with the inference thread waiting for a Metal command buffer. Isolated tests using the cached default GGUF reproduced 30-second generation timeouts under v0.9.5's new automatic paged-attention default, while eager Metal completed both short and retrieval-sized synthetic contexts. Ragtag therefore calls `without_paged_attn()` on macOS, retaining GPU execution, the shared model, and engine-managed concurrency. Other platforms retain their existing defaults. [B199](tracker.md) tracks qualification before removing this compatibility policy.

**Request control and verification:** Local requests set `enable_thinking(false)` instead of relying on prose instructions to prevent hidden reasoning. The application and real-engine examples share the same builder policy and request helper. The `local_generation_probe` example's `app` mode verified first content at approximately 0.09 seconds for a short source and 3.32 seconds for 128 repeated synthetic passages, with cited answers completing at approximately 0.15 and 3.48 seconds. These are diagnostic timings on one host, not a general performance claim or full desktop acceptance.

## D008 — Format-specific extraction with page preservation

**Context:** Citations must identify where retrieved evidence came from across paginated and non-paginated sources.

**Decision:** Extract PDFs with the PDF pipeline, DOCX locally, and other supported sources as UTF-8 text.

**Consequences:** PDF chunks retain physical pages. DOCX and UTF-8 sources use one logical source page. Extraction relies on streaming parser limits without artificial decompression budgets.

**Security maintenance:** Pin the direct DOCX parser to `quick-xml` 0.42.0, retaining the fixes for namespace-allocation and duplicate-attribute denial-of-service advisories RUSTSEC-2026-0195 and RUSTSEC-2026-0194. Retain extraction and preview regression coverage when updating the parser.

**Parser compatibility:** Normalize DOCX XML to UTF-8 before reading quick-xml 0.42 string events. Full extraction rejects invalid UTF-8; bounded previews preserve lossy recovery. Retain the parser's 128 in-scope namespace-binding limit and verify its boundary in tests.

## D009 — Tokenizer-boundary chunking

**Context:** Stored chunks must fit the embedding model without silent truncation and remain traceable across indexing changes.

**Decision:** Split chunks at active tokenizer boundaries and record successful indexing provenance in SQLite.

**Consequences:** The `passage: ` instruction prefix and special tokens count toward embedding input. Changed chunk configuration triggers provenance-based reindexing. [Data schema and IPC contracts](data-schema.md) defines the persisted settings and constraints.

## D010 — Request-scoped streaming

**Context:** Concurrent query and ingestion work requires ordered delivery that stays attached to the initiating request.

**Decision:** Use Tauri v2 channels for answer tokens and ingestion progress. Use application events only for lifecycle compatibility signals.

**Consequences:** Query output and upload progress do not compete in global event listeners. [Data schema and IPC contracts](data-schema.md) defines the event values and failure boundary.

## D011 — First-run model downloads

**Context:** Bundling default model files would increase installer size.

**Decision:** Download default model artifacts on first use rather than bundle them in the installer.

**Consequences:** The application exposes staged availability. App-managed model files can be removed without deleting a user-selected Hugging Face cache. Repositories and GGUF filenames are user-configurable with lightweight defaults.

**Integrity boundary:** Before loading a downloaded or cached GGUF, validate its header and SHA-256 against the Hub's content-addressed cache identity in a cancellable blocking worker. This checks corruption without an online restart probe; it does not authenticate a publisher, pin mutable repository branches, or verify FastEmbed/tokenizer artifacts. Model weights remain runtime downloads, not bundled release components.

**Cache reuse:** Resolve the retained GGUF from the local Hub cache before attempting a download. Verified cache hits do not perform a metadata request, so offline restart does not wait for a network timeout. Existing snapshots remain in use until the user changes model settings or removes the corresponding app-managed cache.

## D012 — Native macOS window chrome

**Context:** Ragtag requires a compact macOS title area while retaining native window controls.

**Decision:** Use Tauri's native macOS overlay titlebar with hidden title text and retained traffic-light controls.

**Consequences:** The WebView supplies the visual header while macOS owns close, minimize, zoom, and drag behavior.

## D013 — Apple Silicon exclusive target

**Context:** Release and development support must align with the tested local-inference stack and Metal acceleration.

**Decision:** Target Apple Silicon (`aarch64-apple-darwin`) exclusively with Metal acceleration. Intel macOS development and external ONNX Runtime workarounds have been retired.

**Consequences:** Build tasks, dependencies, and environments target Apple Silicon and Metal directly. Intel-specific task runners and external ONNX Runtime setup are removed.

**Minimum system version:** Require macOS 15. The pinned `mistralrs-metal-compile` runtime compiler calls `MTLCompileOptions.setMathMode`, which Apple's Metal headers mark as available from macOS 15; it also selects Metal language 3.1 (macOS 14). The current statically linked ONNX Runtime objects target macOS 13.4. Declaring macOS 11 would therefore misrepresent the native stack. This deployment floor is not a substitute for packaged-app acceptance testing on the oldest supported OS.

## D014 — Cross-platform scaffolding

**Context:** The application needs foundational compile-time switches, path normalization, and platform-adaptive UI hooks to prepare for Linux and Windows support without disrupting primary macOS Apple Silicon functionality.

**Decision:** Use target-conditional dependency blocks for `mistralrs` (enabling Metal only on Apple Silicon, with CPU/CUDA fallback on non-macOS), normalize file paths against Windows backslashes and UNC prefixes, expose a typed `get_platform` IPC hook, and adapt frontend header padding dynamically.

**Consequences:** macOS continues to compile with native Metal acceleration while the codebase gains structural portability for future Windows and Linux builds.

## D015 — Resilient external inference and credential fallback

**Context:** Local inference servers (e.g. Ollama, LM Studio, vLLM) frequently operate on loopback or local networks without requiring authentication or TLS encryption. Furthermore, desktop environments may have locked, unconfigured, or unavailable OS keychain services. Enforcing mandatory API credentials or strictly requiring remote HTTPS breaks local developer workflows and causes total credential persistence failures on environments lacking OS keyring support.

**Decision:** Support keyless external inference by sending unauthenticated probing requests when the API key is omitted, forward keys in standard `Authorization: Bearer <key>` headers whenever provided (regardless of local or remote endpoint), and provide automatic non-fatal fallback of credential storage to SQLite (`app_settings.external_api_key`) when the OS keychain is inaccessible, displaying explicit UI indicators and warnings for unencrypted database storage.

**Consequences:** Local inference engines connect out-of-the-box without requiring dummy keys. Environments without functional OS keychains can persist external inference credentials with transparent user notification. Configured credentials are sent in Bearer authorization headers; transport confidentiality requires HTTPS.

## D016 — Modular desktop application architecture

**Context:** As Ragtag grew, `src-tauri/src/lib.rs` expanded into an unwieldy single file (~4,400 LOC) containing IPC command registration, storage operations, inference streaming, model coordinator lifecycle state machines, and ingestion workers. Furthermore, keeping storage validation and reconciliation inside the application shell coupled headless persistence operations to the desktop wrapper.

**Decision:** Decompose `lib.rs` into focused modules (`commands`, `inference`, `model_lifecycle`, `workers`), decouple managed document sandboxing, orphan reconciliation, and maintenance calculations into `crates/persistence::storage`, promote shared limits (`MAX_UPLOAD_BYTES`, `EXTRACTED_TEXT_MAX_BYTES`) to `crates/domain`, and keep `lib.rs` strictly as a lightweight composition and setup root (<180 LOC).

**Consequences:** Clear single-responsibility boundaries, isolated headless unit tests for storage operations without requiring Tauri runtime handles, simplified maintenance, and zero IPC contract breaking changes. All blocking operations remain strictly dispatched to worker threads.

## D017 — Structured error contract with user recovery guidance

**Context:** Application errors previously serialized over IPC as `{ kind, message }`. When operations failed (e.g. queue saturation, missing documents, unconfigured or offline external endpoints, duplicate uploads), error dialogs often displayed raw technical strings without actionable recovery steps, and frontend code could not reliably distinguish transient/retryable failures from permanent user-actionable errors.

**Decision:** Standardize `AppError` and the IPC error payload on `{ code, operation, detail, recovery, message }`. Use a typed `ErrorCode` enum serialized as snake_case (`not_found`, `invalid_input`, `queue_full`, `duplicate_document`, `storage`, `vector_db`, `inference`, `internal`). Enforce non-empty active-voice user recovery guidance on every error constructor (`DEV-04` and `DEV-14`). The frontend error dialog renders explicit transient status badges and dedicated suggested-action guidance panels while preserving backwards compatibility through the formatted `message` string field.

**Consequences:** Users receive clear diagnostic feedback paired with direct instructions on how to recover from any error. Frontend code deterministically classifies retryable operations (`queue_full`, `inference`, `storage`) from terminal failures. Raw strings and untyped errors across backend and frontend are eliminated.

## D018 — Protected execution capacity and memory-only lifecycle probes

**Context:** High-throughput batch operations (such as multi-document ingestion queues or long-running embedding calculations) and active model generation occupy execution capacity. Under load, interactive commands such as cancellation requests (`cancel_query`, `cancel_model_download`) and frontend lifecycle polling can compete for that capacity. The frontend also needs a readiness probe on startup or reconnect rather than relying only on lifecycle events.

**Decision:** Partition command capacity using two semaphores:
1. **Partitioned capacity (`ExecutionCapacity`):** Divide Tauri command execution into two dedicated pools:
   - `ORDINARY_COMMAND_CAPACITY` (16 permits): Guards bulk, mutation, and inference commands (`upload_document`, `execute_query`, `delete_document`, `list_documents`, `get_document_preview`, settings, maintenance). Saturated capacity immediately returns `ErrorCode::QueueFull` with active-voice retry guidance.
   - `PROTECTED_COMMAND_CAPACITY` (4 permits): Reserved strictly for high-priority interactive operations (`cancel_query`, `cancel_model_download`, `get_app_state`). Protected commands never queue behind ordinary operations.
2. **Memory-only lifecycle probe (`get_app_state`):** Return embedder readiness, LLM state, in-flight download progress, active-query/ingestion state, and capacity permit availability without querying SQLite or LanceDB. Fields are read under separate locks; this is not an atomic snapshot or a latency guarantee.
3. **Cancellable model downloads:** Wrap HTTP download streams in Tokio watch cancellation channels, allowing users to abort multi-gigabyte background downloads immediately and enter the app as soon as document embedding is ready.

**Consequences:** Ordinary command saturation does not consume protected permits for cancellation and lifecycle polling. Runtime scheduling and lock contention can still affect response latency.

## Dependency compatibility

The 2026-11-26 update targets stable Rust 1.100.0, FastEmbed 7.8.0, tokenizers 0.23.2, SQLx 0.9.1, and LanceDB 0.41.0. FastEmbed requires ONNX Runtime 2.0.0-rc.15. Preserve exact pins for this native compatibility group and keep the resolved transitive updates in `Cargo.lock`.

Jiff 1.0.3 renames `Zoned` to `ZonedDateTime`; document creation retains the existing local-time `%Y-%m-%d %H:%M:%S` storage format.

LanceDB 0.41 uses Arrow 58, so direct record-batch types use its latest compatible patch, 58.4.2, rather than Arrow 60. Earlier LanceDB 0.38–0.40 releases failed embedded-only compilation by referencing remote-only `Error::Http`; 0.41 adds a feature-gated runtime-error fallback. Keep embedded-only operation and do not enable unused remote features to bypass such defects.

Frontend dependencies use the latest stable releases checked on that date, except TypeScript: `svelte-check` 4.7.8 declares support for TypeScript 5 and 6, not 7. Retain TypeScript 6.0.3 until its checker supports the newer major; do not override peer checks. Updated Svelte components use typed destructuring with `$props()` and module-script type exports.
