# Ragtag Watchdog Notes

Advisor-only guidance. Review implementing work against `AGENTS.md` and the focused documents in `docs/`. Catch a wrong direction while it is cheap to correct. Do not become a second implementer.

## Severity

- **Blocker:** Continuing would violate a local-first product invariant, put user documents or secrets at risk, or create an architectural dependency that must be unwound before distribution.
- **Concern:** The change materially risks a dropped constraint, broken ingestion/retrieval integrity, an invented API, or a design that conflicts with the documented architecture.
- **Nit:** Cleanup, naming, or a smaller edge case. Batch these. Do not interrupt work for a nit.

## Block Immediately

- **The change adds an internal HTTP server, hosted backend, inference sidecar, or required cloud service.** Ragtag is a single local Tauri application. Default embeddings and generation run in-process; a user-configured external OpenAI-compatible endpoint is the only intended remote inference path.
- **`domain` imports Tauri, `sqlx`, LanceDB, filesystem, `reqwest`, `mistralrs`, `fastembed`, or UI/provider code.** `domain` contains portable shared types and errors. This reverses the adapter boundary.
- **A secret enters SQLite, frontend storage, an event, or a log.** External API keys belong exclusively in the OS keychain through `keyring`.
- **A change treats LanceDB chunks, generated answers, or model output as canonical document evidence.** Original PDFs and the SQLite document registry remain authoritative; the vector store is derived data.
- **Indexing marks a document indexed before its vector upsert succeeds, or loses stable page/chunk identity.** This breaks crash recovery, idempotence, and citation integrity.
- **Checks or tests are weakened to obtain green output.** Fix the behavior or surface the conflict. Never hide it through a skipped test, suppression, or reduced build strictness.

## Raise a Concern

- **Tauri APIs appear outside `frontend/src/lib/api.ts`.** Components and stores must use typed functions from that module, never invoke IPC directly.
- **Commands or `src-tauri/src/` acquire new business, storage, or retrieval logic that belongs in a workspace crate.** The app layer composes state and adapts IPC; library crates own reusable behavior.
- **Blocking PDF, filesystem, or embedding work runs directly inside an async command.** Move it to `spawn_blocking`, `tokio::fs`, or an owned background worker.
- **A command uses `send().await` to enqueue ingestion work.** Commands must use `try_send()` so a full queue returns a typed error instead of freezing the UI.
- **The change adds SvelteKit, SSR, Tailwind, a router framework, a global state package, an internal API server, or an unreviewed dependency.** These conflict with deliberate desktop-SPA and local-first choices unless the relevant design document changes first.
- **The retrieval contract changes without a complete path review.** `passage: ` and `query: ` prefixes, 768-dimension embeddings, page-preserving extraction, deterministic chunk IDs, LanceDB upserts, hybrid search, and citation payloads must agree end-to-end.
- **A model-generated page number is presented as a citation.** Sources must originate from retrieved chunk metadata.
- **An external API key is deleted then reset, or otherwise handled outside keychain operations.** Use keyring upsert semantics; never persist credentials as settings.
- **A v1 Tauri import/configuration pattern or unpinned fragile native dependency is introduced.** Verify current APIs and preserve declared version constraints.

## Known Traps

- **First-run readiness has two stages.** The embedder unlocks ingestion; local generation stays unavailable until the model is downloaded and built. External inference deliberately bypasses the local LLM requirement.
- **PDF extraction must preserve physical pages.** Page number is a citation contract, not presentation metadata. Chunking may estimate token size, but it must never merge page provenance away.
- **LanceDB requires write serialization and index lifecycle care.** Use idempotent `merge_insert`; build or rebuild FTS only after rows exist; handle the documented vector-only fallback without silently losing search diagnostics.
- **Local PDF display is capability-scoped.** Construct asset URLs through Tauri's asset protocol and retain the bundled PDF.js path for WebView consistency.
- **SQLite is persistent product state, not a disposable cache.** Document registry, statuses, and settings must migrate safely. LanceDB is rebuildable derived search data, but reset/delete flows must preserve their documented ordering and explicit confirmation semantics.
- **Generation cancellation is request-scoped.** `cancel_query` must interrupt external readiness and provider streams or local stream reads; it cannot stop already-running blocking embedding work. Do not substitute frontend-only late-event suppression for backend cancellation.
- **Settings must reflect enforced behavior.** Do not expose a new setting or treat an existing setting as effective unless the backend path reads and enforces it.

## Process Drift

- **Unverified completion:** no targeted build, test, or exercised desktop path after a behavior change. Concern.
- **Scope creep:** a small request expands into unrelated stack, dependency, model, or product work. Nit first, concern if the scope continues to grow.
- **Invented API:** a plausible crate, model, Tauri, or LanceDB method appears without source verification. Concern before more code is built on it.
- **Silent narrowing:** friction causes an explicit requirement, storage transition, or citation edge case to disappear. Concern and name the omitted requirement.
- **Documentation drift:** a behavior or architecture decision changes without its focused document or tracker entry being updated. Concern.

## Do Not Raise

- Formatting or naming covered by existing tooling.
- Work that is still incomplete but has no demonstrated invariant or architecture violation.
- A point already explained and deliberately rejected with evidence.
- Speculative future-proofing outside the request.
