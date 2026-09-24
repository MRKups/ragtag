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

**Consequences:** Default processing, retrieval, and generation need no model server; external inference crosses the configured provider boundary. The key remains in the OS keychain, and [B186](tracker.md) tracks the transport restriction.

**Dependency source:** Pin `mistral.rs` v0.9.3 to Git revision `24dbf5c256f232176ee5949485ba264049407fbe` with Metal acceleration on Apple Silicon. At the 2026-09-21 review, crates.io's latest SDK was 0.8.1. Use the source revision rather than a floating branch or tag, and retain the application's resolved `Cargo.lock`, including Candle 0.11.0 at Git revision `35d7ae7ca5c93e17c77359c3617376b8a72e96a4`.

**Maintained deviation:** Override only `mistralrs-core` with the licensed source in `src-tauri/vendor/mistralrs-core`. Patch `ragtag.1` makes narrowed batched logits selections contiguous before quantized projection; it preserves concurrent inference rather than adding a request semaphore. The vendored manifest resolves upstream workspace inheritance and keeps sibling packages on the same Git revision. Its build script reports the upstream revision plus `ragtag.1`, not the enclosing application's Git HEAD. All other upstream source remains unchanged.

**Maintenance boundary:** Keep the imported package and license intact, with the manifest normalization, revision reporting, and selection repair as explicit deviations. Preserve the two original fixtures in the adjacent `vendor/toml-selectors/`; the upstream crate needs them even when compiling a filtered test target. Its focused regression is part of the native test tasks; the rest of the upstream suite is not implicitly part of Ragtag's workspace. Retire the override only after an upstream replacement provides the repair and equivalent regression and real-model qualification pass. [README](../README.md#build-profile-and-upstream-deviations) explains the build inputs and platform requirements without treating compilation as runtime acceptance.

**Qualification:** The patched engine was qualified for batched logits selection contiguousness before quantized projection, supporting concurrent queries. [B189](tracker.md) retains the Apple Silicon Metal and packaged-app acceptance gate.

## D008 — Format-specific extraction with page preservation

**Context:** Citations must identify where retrieved evidence came from across paginated and non-paginated sources.

**Decision:** Extract PDFs with the PDF pipeline, DOCX locally, and other supported sources as UTF-8 text.

**Consequences:** PDF chunks retain physical pages. DOCX and UTF-8 sources use one logical source page. [B185](tracker.md) records the unresolved PDF decompression-allocation limit.

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

**Consequences:** The application exposes staged availability. App-managed model files can be removed without deleting a user-selected Hugging Face cache. [B187](tracker.md) tracks artifact pinning and verification.

## D012 — Native macOS window chrome

**Context:** Ragtag requires a compact macOS title area while retaining native window controls.

**Decision:** Use Tauri's native macOS overlay titlebar with hidden title text and retained traffic-light controls.

**Consequences:** The WebView supplies the visual header while macOS owns close, minimize, zoom, and drag behavior.

## D013 — Apple Silicon exclusive target

**Context:** Release and development support must align with the tested local-inference stack and Metal acceleration.

**Decision:** Target Apple Silicon (`aarch64-apple-darwin`) exclusively with Metal acceleration. Intel macOS development and external ONNX Runtime workarounds have been retired.

**Consequences:** Build tasks, dependencies, and environments target Apple Silicon and Metal directly. Intel-specific task runners and external ONNX Runtime setup are removed.

## D014 — Cross-platform scaffolding

**Context:** The application needs foundational compile-time switches, path normalization, and platform-adaptive UI hooks to prepare for Linux and Windows support without disrupting primary macOS Apple Silicon functionality.

**Decision:** Use target-conditional dependency blocks for `mistralrs` (enabling Metal only on Apple Silicon, with CPU/CUDA fallback on non-macOS), normalize file paths against Windows backslashes and UNC prefixes, expose a typed `get_platform` IPC hook, and adapt frontend header padding dynamically.

**Consequences:** macOS continues to compile with native Metal acceleration while the codebase gains structural portability for future Windows and Linux builds.
