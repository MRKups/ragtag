---
title: Ragtag implementation tracker
normative-for: [unresolved implementation work]
depends: [app-architecture.md, data-schema.md, decisions.md, open-source-readiness.md, system-map.md, development.md]
status: current work only. Completed and superseded items are removed.
---

# Ragtag implementation tracker

Items state unresolved outcomes. Current behavior belongs in linked references.

## B189 — Qualify the pinned engine on Apple Silicon

**Problem:** Packaged Apple Silicon application verification requires end-to-end qualification of Metal execution.

**Outcome:** The pinned inference stack is exercised on an Apple Silicon host using the existing native test, release-build, and bundle tasks.

**Acceptance:** Launch the packaged app and verify actual Metal model loading, streamed completion with citations, cancellation and subsequent queries, concurrent queries, cached offline restart, and provider switching. Record startup, first-token latency, generation performance, and peak memory against a same-model baseline; review material regressions before accepting them.

**References:** [D007 and D013](decisions.md), [README build instructions](../README.md), [release gate](open-source-readiness.md).

## B193 — Hybrid retrieval relevance and BM25 re-ranking fine-tuning

**Problem:** Candidate chunk retrieval relevance varies across dense technical manuals containing tables, code, and abbreviations when relying on standard BM25 and vector score blending without domain tuning.

**Outcome:** Benchmark LanceDB hybrid retrieval with tuned lexical/vector weighting, score filtering thresholds, and acronym expansions to maximize citation precision.

**Acceptance:** Measured retrieval precision and recall on technical evaluation documents improves over the baseline without exceeding chunk retrieval latency budgets.

**References:** [DEV-11](development.md), [decisions.md](decisions.md).

## B194 — Resume initialization after settings cancel a download

**Problem:** Changing model settings during download or verification can leave the cancelled generation marked active, preventing the requested restart.

**Outcome:** Stale download failures release their owned generation and schedule one restart using the latest persisted settings, without publishing stale errors.

**Acceptance:** Exercise settings changes during downloading and verification, explicit cancellation, and stale completion after a replacement starts. Preserve the generation and readiness contracts in [D011](decisions.md#d011--first-run-model-downloads).

## B195 — Distinguish rejected settings writes from post-commit queue saturation

**Problem:** The frontend treats every chunk-setting `queue_full` error as a committed change, including command-capacity rejection before persistence.

**Outcome:** Report pre-commit rejection as an unsaved setting while retaining successful settings whose reindex work is draining in the background.

**Acceptance:** Verify both failure stages, persisted values, rollback, and preservation of newer drafts. See [settings](data-schema.md#app_settings) and [execution capacity](data-schema.md#execution-capacity-and-lifecycle-state).

## B196 — Handle failed ingestion-status persistence

**Problem:** Ingestion discards errors while marking a document failed, then sends terminal notifications despite an unchanged queued or embedding registry state.

**Outcome:** Preserve processing diagnostics and recover a failed terminal-state write without treating missing or deleting documents as new failures.

**Acceptance:** Inject registry-write failure and verify notification, retry accounting, and recovery after restart. See [document lifecycle](app-architecture.md#document-lifecycle).

## B197 — Preserve document-failure recovery actions across delivery paths

**Problem:** Channel and lifecycle failure notices share a deduplication key but differ in their recovery action; delivery order determines whether “Open documents” appears.

**Outcome:** Both entry points report a consistent notice while retaining channel progress, background-job lifecycle events, and delayed-progress suppression.

**Acceptance:** Verify both delivery orders, queued notices, dismissal timing, and background-only failures. See [request-scoped events](data-schema.md#request-scoped-events).
