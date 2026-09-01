# Ragtag Implementation Tracker

*Active work only. Completed and superseded entries are removed; architecture and design decisions remain documented in `docs/`.*

*Global ID Counter: Next item is **B139***

## Features (F)
- [ ] F31: Finish granular document-ingestion progress streaming. The implementation emits `ingestion_progress` stage events (`extracting`, `chunking`, `embedding`, `storing`), not the promised per-page/per-chunk `Channel<T>` progress; preserve the existing stage events while adding the required granularity.
- [ ] F32: Fully implement View Transitions between all 3 pages — verify smooth cross-fade animations on hash-based navigation for query/documents/settings routes. Ensure no visual flashes, layout shifts, or header flicker during transitions. The `startViewTransition` feature detection guard is in place; remaining work is full QA across routes.

## Bugfixes (B)
