---
title: Development rules
normative-for: [development invariants, architectural rules, code review standards]
depends: [documentation-style.md, development-process.md]
status: current reference. Immutable rule IDs DEV-01 through DEV-18.
---

# Development rules

Ragtag enforces numbered, immutable engineering rules. Every code modification, review, and automated audit cites these identifiers directly.

---

## Simple

### DEV-01 — One home for every concept
Every rule, contract, and architectural invariant lives in exactly one authoritative document. Code or documentation that touches an existing concept must link to its authoritative specification rather than paraphrasing or duplicating it.

### DEV-02 — Avoid unnecessary abstractions
Prefer direct, readable implementations over speculative generic layers, macro frameworks, or premature plugin architectures. Write code for the concrete capabilities Ragtag requires today.

### DEV-03 — Keep the domain boundary pure
`crates/domain` must remain completely free of Tauri, SQLite, LanceDB, Tokio, Arrow, filesystem, HTTP networking, and UI dependencies. It defines shared domain models, events, and typed errors only.

### DEV-04 — Direct active prose
Write documentation, error messages, and UI text in direct, active voice. Eliminate filler and promotional words ("essentially", "simply", "robust", "seamless", "leverage"). State what happened, what failed, and what the safe recovery is.

---

## Modular

### DEV-05 — Unidirectional workspace dependency flow
Dependencies strictly flow downward:
```text
frontend -> Tauri shell -> domain, persistence, rag_pipeline
rag_pipeline -> domain, persistence
persistence -> domain
```
No crate or module may introduce an upward or circular dependency.

### DEV-06 — Strict frontend IPC boundary
Tauri APIs (`@tauri-apps/api/core` and plugins) are imported exclusively inside `frontend/src/lib/api.ts`. Svelte components and rune stores interact with native features through typed wrapper functions only.

### DEV-07 — Isolate blocking operations
Never execute blocking filesystem I/O, PDF extraction, tokenizer operations, or embedding generation on an async Tauri command handler or Tokio runtime thread. Dispatch blocking work to `tokio::task::spawn_blocking` or dedicated background workers.

### DEV-08 — Decouple core logic from the desktop shell
Keep persistence, extraction, chunking, and embedding logic in reusable workspace crates (`domain`, `persistence`, `rag_pipeline`). Keep `src-tauri/src/` focused on application bootstrapping, lifecycle coordination, and Tauri IPC adaptation.

---

## Reliable

### DEV-09 — Local-first by default with canonical source evidence
Ragtag operates locally without mandatory cloud services or HTTP servers. Uploaded user documents are immutable evidence retained byte-for-byte in application-local storage. Chunks, vectors, and indexes are derived and rebuildable.

### DEV-10 — SQLite owns registry identity and configuration
SQLite is the single source of truth for document identity, processing lifecycle status, indexing provenance, and non-secret settings. LanceDB holds derived vectors and full-text indexes and does not own document lifecycle states.

### DEV-11 — Retrieval-grounded citations
Citations must originate from retrieved chunk metadata (document ID, document title, physical page number). Never present model-generated text or hallucinated page numbers as citations.

### DEV-12 — Idempotent and crash-safe indexing
Per-document chunk identifiers are deterministic: `(document_id, page, chunk_index)`. Vector writes are serialized using `merge_insert`. A document is marked `indexed` in SQLite only after vector insertion succeeds. Startup recovery reconciles interrupted or orphaned jobs.

### DEV-13 — Isolated secrets management
External API keys are stored primarily in the OS keychain via `keyring`. If the keychain is unavailable, credentials fall back to unencrypted SQLite storage (`app_settings.external_api_key`) with explicit user notification. Credentials must never enter frontend storage, logs, events, or diagnostics.

### DEV-14 — Structured and serializable errors
Every command error returned over IPC must be strongly typed and serializable via `AppError`. Never return unstructured strings or raw panic payloads across the IPC bridge.

### DEV-15 — Zero suppression of compiler and test checks
Never suppress compiler warnings, add `#[allow(...)]`, ignore tests with `#[ignore]`, or weaken test assertions to achieve a green build. Fix the underlying issue or revise the documented invariant.

---

## Working Process

### DEV-16 — Consult authoritative documentation before editing
Read the relevant design document (`docs/app-architecture.md`, `docs/data-schema.md`, `docs/decisions.md`, `docs/system-map.md`) before changing code or architecture. Treat documented design decisions as deliberate.

### DEV-17 — Maintain documentation with implementation
Update affected documentation and data schema specifications in the same commit as the code change that alters the contract. Keep `docs/file-structure.md` and `docs/system-map.md` aligned with repository reality.

### DEV-18 — Discrete atomic commits with explicit staging
Make one commit for one reviewable intent. Stage explicit file paths only (`git add <file>`), never using `git add -A` or committing unreviewed user modifications. Verify builds and tests before committing.
