# Ragtag Agent Instructions

Ragtag is a local-first desktop RAG application for querying user-provided PDFs and technical manuals. It must keep documents, indexes, models, and default inference on the user's machine while returning answers with retrieval-grounded page citations.

## Read First

Before changing implementation or architecture, read:

- `docs/app-architecture.md`, the system boundaries and data flows
- `docs/data-schema.md`, the SQLite, LanceDB, settings, and IPC contracts
- `docs/decisions.md`, the rationale for deliberate technology choices
- `docs/file-structure.md`, the repository map and ownership boundaries
- the relevant unfinished or follow-up item in `docs/tracker.md`

Treat an explicit unresolved tracker item or a documented design decision as intentional. Do not silently replace an established approach while implementing adjacent work.

## Finding Code

Use `docs/file-structure.md` to locate directories and entry points. It is a navigation aid; `docs/app-architecture.md`, `docs/data-schema.md`, and `docs/decisions.md` remain authoritative for behavior, storage, and architectural decisions.

## Repository Layout

```text
frontend/                         Svelte 5 and Vite desktop UI
  src/lib/api.ts                  only frontend boundary allowed to import Tauri APIs
  src/pages/                      query, document-management, and settings pages
src-tauri/                        Tauri app shell and Rust workspace
  src/                            composition, model download, state, and IPC commands
  crates/domain/                  shared models, typed errors, and token events
  crates/persistence/             SQLite repositories and embedded LanceDB storage
  crates/rag_pipeline/            PDF extraction, chunking, embeddings, and indexing
```

The intended dependency direction is:

```text
frontend -> Tauri commands/app composition -> domain, persistence, rag_pipeline
rag_pipeline -> domain, persistence
persistence -> domain
```

`domain` must stay free of Tauri, SQLite, LanceDB, filesystem, HTTP, model-provider, and UI dependencies. Keep new document-processing, retrieval, and persistence behavior in the library crates; `src-tauri/src/` composes state, adapts IPC, and coordinates application lifecycle.

## Product Invariants

- **Local-first is the default.** Do not add an internal HTTP server, cloud backend, sidecar, or required hosted service. Embeddings and default generation run in-process; external OpenAI-compatible inference is an explicit user-selected exception.
- **Original documents are user-owned evidence.** Store uploaded PDFs in application-local data without rewriting their content. Do not make a vector chunk, generated answer, or model output the canonical source document.
- **SQLite owns the registry and settings.** `documents` is the source of truth for document identity and processing state; `app_settings` owns non-secret configuration. LanceDB stores derived chunks, vectors, and search indexes and must remain recoverable from retained documents.
- **Secrets stay in the OS keychain by default.** `external_api_key` falls back to unencrypted SQLite storage (`app_settings`) only when keychain services are unavailable. It must never enter frontend storage, logs, events, or diagnostics.
- **Citations are retrieval-grounded.** Citation page and document metadata come from retrieved chunks, not model-generated page numbers. Preserve page boundaries through extraction, chunking, indexing, search, and IPC.
- **Indexing is crash-safe and idempotent.** Keep per-document chunk keys stable, upsert vectors before marking SQLite records indexed, and preserve startup recovery for queued or interrupted embedding jobs.
- **Inference is staged.** Document ingestion unlocks only after the embedder is ready; local querying waits for the local model, while configured external inference does not require a local LLM download.

## Frontend Rules

- Use Svelte 5 components, semantic HTML, plain component-scoped CSS, and the existing Svelte 5 rune stores.
- Keep every Tauri API import inside `frontend/src/lib/api.ts`; components and stores call typed API functions only.
- Preserve the plain Vite SPA: hash routing, client-only execution, and `document.startViewTransition()` where applicable. Do not add SvelteKit, SSR, a router framework, global state library, Tailwind, or a component kit.
- Use Tauri v2 APIs only: IPC comes from `@tauri-apps/api/core`; use the matching v2 plugin package for plugin capabilities. Do not introduce v1 paths such as `@tauri-apps/api/tauri`, `@tauri-apps/api/fs`, or `@tauri-apps/api/window`.
- Render model output only through the existing sanitization path. Keep PDF rendering through the asset protocol and bundled PDF.js path; do not expose arbitrary filesystem paths to the WebView.
- Stream request-scoped answers and ingestion progress through Tauri v2 `Channel<T>` callbacks. Reserve application events for lifecycle and readiness broadcasts; do not turn ordered response streaming into global event traffic.

## Rust Rules

- Preserve the workspace boundaries. A dependency from `domain` to an adapter is a design error; a new cross-crate dependency needs a concrete documented reason.
- Treat `src-tauri/Cargo.toml` and each crate manifest as the source of truth for versions. Preserve exact pins where declared, especially `sqlx`, `fastembed`, `mistralrs`, LanceDB, and Arrow compatibility.
- Use Tauri v2 APIs and configuration. Do not introduce v1 import paths, `distDir`/`devPath` config, or legacy window APIs.
- Use `sqlx` with SQLite migrations; do not add an ORM. Create the application-local data directory before opening the pool, preserve the committed `.sqlx/` offline-query cache when SQLx macros are used, and keep app settings in SQLite.
- Store external API keys primarily in `keyring` (with SQLite fallback if unavailable); use its upsert semantics rather than a delete-then-set sequence.
- Run blocking filesystem, PDF extraction, embedding initialization, and embedding work in `spawn_blocking` or an appropriate blocking worker. Do not block an async Tauri command or the runtime executor.
- Use `std::sync::Mutex` unless a lock must survive an `.await`; then use Tokio synchronization deliberately. The shared `mistralrs::Model` is `Arc`-shared and must not gain a mutex.
- Enqueue document work from commands with `try_send()` and report typed queue-full errors. Background producers may await `send()` for backpressure.
- Keep the 768-dimensional EmbeddingGemma contract, instruction prefixes (`passage: ` and `query: `), deterministic chunk identifiers, LanceDB `merge_insert`, serialized vector writes, and FTS index creation after rows exist unless the retrieval design is deliberately changed.
- Build the local model once, retain it in shared application state, and let `mistralrs` manage generation concurrency. Keep Metal conditional on macOS; do not spawn a model server or add a request semaphore in front of `stream_chat_request`.
- For macOS signing or notarization, treat `onnxruntime.dylib` as a bundled native dependency that needs hardened-runtime signing and the library-validation entitlement; `mistralrs` is statically linked.
- Do not race a SQLx transaction-acquisition future against a timeout or `tokio::select!` branch: dropping it can poison connection transaction state. Resolve the operation or rebuild the affected pool.
- Command errors must remain typed and serializable through `AppError`; do not return opaque `String` errors over IPC.

## Git Hygiene

- Make one commit for one complete, reviewable intent. Do not mix unrelated cleanup, generated output, or another task's work into it.
- Include implementation, changed-contract verification, and affected documentation in the same commit.
- Before staging, inspect `git status`. Stage explicit paths only; never use `git add -A` or commit unreviewed user changes.
- Before committing, inspect the staged diff and run the narrowest meaningful verification. Exclude models, build output, databases, operating-system artifacts, and other generated files.
- Use a short imperative commit subject, for example `Add document retry status`.
- Do not amend, rebase, reset, force-push, or alter existing commits without explicit user direction.
- Prepare a verified commit boundary when a requested unit is complete. Commit and push only when explicitly requested; report the commit ID, remote result, and remaining working-tree changes.

## Working Rules

- Read the focused design document before editing. Reuse existing patterns rather than creating a second convention.
- Keep changes scoped to the requested behavior. Delete code made obsolete by the same change.
- Do not weaken checks, add suppressions, or change build settings to make a failure disappear. Diagnose and fix the cause.
- Verify changed behavior using the narrowest meaningful build, test, or exercised desktop scenario. Do not claim completion without its result.
- Update the focused design document when implementation settles or changes an architectural or product decision. Keep `docs/file-structure.md` and `docs/tracker.md` accurate when layout or tracked implementation status changes.
