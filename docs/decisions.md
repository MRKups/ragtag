---
title: Design rationale
normative-for: [durable architecture and technology decisions]
depends: [app-architecture.md, data-schema.md]
status: current decision record. Source code and migrations implement the accepted decisions.
---

# Design rationale

This document records durable choices and their tradeoffs. Current behavior belongs in the linked reference documents. Each record is accepted unless it names a later superseding record.

## D001 — Tauri desktop application

**Status:** Accepted

**Context:** Ragtag must run as a local desktop application without a required hosted service.

**Decision:** Use Tauri with a Svelte interface and a Rust application layer.

**Consequences:** The application ships as a native desktop bundle. The WebView uses typed IPC for native work. Local operating-system capabilities remain explicit Tauri concerns.

## D002 — Tauri IPC boundary

**Status:** Accepted

**Context:** The interface needs access to document, retrieval, and settings operations without becoming a native-storage client.

**Decision:** Use Tauri v2 commands and request-scoped channels for frontend-to-Rust communication.

**Consequences:** The frontend imports Tauri APIs only through `frontend/src/lib/api.ts`. [Data schema and IPC contracts](data-schema.md) defines the wire vocabulary.

## D003 — SQLite document registry

**Status:** Accepted

**Context:** Document identity, lifecycle state, and settings must survive restarts and coordinate recoverable derived data.

**Decision:** Use embedded SQLite as the registry and settings store.

**Consequences:** SQLite is the authority for document retrieval eligibility and indexing provenance. [Data schema and IPC contracts](data-schema.md) defines the effective schema.

## D004 — In-memory document work queue

**Status:** Accepted

**Context:** Document processing is asynchronous but depends on durable registry state rather than an external work service.

**Decision:** Use an in-process bounded queue for document work, with startup recovery from SQLite state.

**Consequences:** The queue improves interactive responsiveness but is not the source of truth. Queued and interrupted work remains recoverable from the registry.

## D005 — Application-local managed sources

**Status:** Accepted

**Context:** User documents are evidence for citations and must remain distinct from generated output and indexes.

**Decision:** Retain managed copies in application-local storage and treat them as canonical evidence.

**Consequences:** Chunks, vectors, previews, and answers are derived data. They can be removed and rebuilt from managed sources.

## D006 — Embedded LanceDB retrieval

**Status:** Accepted

**Context:** Ragtag needs local vector and full-text retrieval without a separate database service.

**Decision:** Use embedded LanceDB for derived chunks and vectors.

**Consequences:** LanceDB remains recoverable from SQLite and managed sources. It does not own document lifecycle state. Search may degrade from hybrid to vector-only retrieval, as described in [Application architecture](app-architecture.md).

## D007 — Local in-process inference by default

**Status:** Accepted

**Context:** Default answers and embeddings must remain local while allowing an explicit provider choice for users who need external inference.

**Decision:** Run FastEmbed embeddings and `mistral.rs` generation in the application process by default. Support user-selected OpenAI-compatible external inference as an exception.

**Consequences:** Default document processing, retrieval, and generation require no model server. External inference sends the query and retrieved excerpts to the configured provider. The API key remains in the OS keychain. [B186](tracker.md) records the pending transport restriction.

## D008 — Format-specific extraction with page preservation

**Status:** Accepted

**Context:** Citations must identify where retrieved evidence came from across paginated and non-paginated sources.

**Decision:** Extract PDFs with the PDF pipeline, DOCX with local ZIP/XML processing, and other supported sources as UTF-8 text.

**Consequences:** PDF chunks retain physical pages. DOCX and UTF-8 sources use one logical source page. [B185](tracker.md) records the unresolved PDF decompression-allocation limit.

## D009 — Tokenizer-boundary chunking

**Status:** Accepted

**Context:** Stored chunks must fit the embedding model without silent truncation and remain traceable across indexing changes.

**Decision:** Split chunks at active tokenizer boundaries and record successful indexing provenance in SQLite.

**Consequences:** The `passage: ` instruction prefix and special tokens count toward embedding input. Changed chunk configuration triggers provenance-based reindexing. [Data schema and IPC contracts](data-schema.md) defines the persisted settings and constraints.

## D010 — Request-scoped streaming

**Status:** Accepted

**Context:** Concurrent query and ingestion work requires ordered delivery that stays attached to the initiating request.

**Decision:** Use Tauri v2 channels for answer tokens and ingestion progress. Use application events only for lifecycle compatibility signals.

**Consequences:** Query output and upload progress do not compete in global event listeners. [Data schema and IPC contracts](data-schema.md) defines the event values and failure boundary.

## D011 — First-run model downloads

**Status:** Accepted

**Context:** Bundling default model files would materially increase the application bundle size.

**Decision:** Download default model artifacts on first use rather than bundle them in the installer.

**Consequences:** The application exposes staged availability. App-managed model files can be removed without deleting a user-selected Hugging Face cache. [B187](tracker.md) records the pending immutable revision and checksum verification.

## D012 — Native macOS window chrome

**Status:** Accepted

**Context:** Ragtag requires a compact macOS title area while retaining native window controls.

**Decision:** Use Tauri's native macOS overlay titlebar with hidden title text and retained traffic-light controls.

**Consequences:** The WebView supplies the visual header while macOS owns close, minimize, zoom, and drag behavior.

## D013 — Apple Silicon release target

**Status:** Accepted

**Context:** Release support must align with the tested local-inference stack.

**Decision:** Release and bundle Apple Silicon builds. Permit Intel macOS only as an isolated development and test path with an external ONNX Runtime.

**Consequences:** Intel runtime material remains outside the repository and release bundle. [README](../README.md) owns setup instructions.
