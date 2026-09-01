# Ragtag Implementation Tracker

*Active work only. Completed and superseded entries are removed; architecture and design decisions remain documented in `docs/`.*

*Global ID Counter: Next item is **B138***

## Features (F)
- [ ] F31: Finish granular document-ingestion progress streaming. The implementation emits `ingestion_progress` stage events (`extracting`, `chunking`, `embedding`, `storing`), not the promised per-page/per-chunk `Channel<T>` progress; preserve the existing stage events while adding the required granularity.
- [ ] F32: Fully implement View Transitions between all 3 pages — verify smooth cross-fade animations on hash-based navigation for query/documents/settings routes. Ensure no visual flashes, layout shifts, or header flicker during transitions. The `startViewTransition` feature detection guard is in place; remaining work is full QA across routes.

## Bugfixes (B)
> **F10 reconciliation (2026-08-31):** B112, B121, B132, B133, B134, and B137 remain active follow-ups. `docs/review-2026-08-31.md` records completed remediation of the earlier confirmed findings, not completion of these six broader contracts.

- [ ] B52: Consolidate local `mistralrs` and external `reqwest` streaming behind a shared inference abstraction. They share retrieval, prompt construction, and citation assembly today, but `execute_query` still contains separate local and external stream loops rather than the claimed trait/enum dispatch.
- [ ] B112: Restore a recoverable local-model download failure path. A download error emits `model_download_progress` with an error and does not emit `llm_ready`; once the embedder is ready the Settings UI remains available, but query stays disabled and changing settings does not restart initialization.
- [ ] B121: Provide a recoverable local-model build failure path. A `GgufModelBuilder` error emits `model_download_progress` with an error and does not emit `llm_ready`; the query UI remains unavailable until initialization is retried.
- [ ] B132: Strengthen page-aware reindex accounting. A global `extraction_version` mismatch requeues every indexed document after the embedder becomes ready, but no extraction version is stored with individual documents or vector rows, so the application cannot show or verify per-document migration completion.
- [ ] B133: Filter vector candidates by document lifecycle before applying the retrieval limit. `execute_query` discards non-`indexed` documents only after LanceDB returns the top-K rows, so queued, failed, deleted-race, or reindexing chunks can consume all retrieval slots.
- [ ] B134: Validate external inference readiness before enabling queries. Inference-mode changes now transition readiness and initialize local inference when needed, but external mode still reports ready before verifying its endpoint, model, or key.
- [ ] B137: Persist chunking configuration/version with each document and reindex affected documents when chunk settings change. Tokenizer-boundary chunks now prevent silent embedding truncation, but no per-document configuration/version is stored and changing `chunk_size` or `chunk_overlap` does not reindex existing documents.
