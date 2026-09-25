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

## B192 — Protected execution capacity for cancellations and lifecycle probes

**Problem:** Saturated background ingestion queues or long-running embedding batches can create execution latency for interactive user commands, potentially delaying cancellation requests and status polling.

**Outcome:** Implement protected command capacity separating high-priority interactive operations (`cancel_query`, `cancel_model_download`, `get_app_state`) from batch background processing.

**Acceptance:** A saturated document ingestion queue or active inference generation does not delay execution of cancellation requests or status queries beyond bounded scheduling limits.

**References:** [DEV-07](development.md), [app-architecture.md](app-architecture.md).

## B193 — Hybrid retrieval relevance and BM25 re-ranking fine-tuning

**Problem:** Candidate chunk retrieval relevance varies across dense technical manuals containing tables, code, and abbreviations when relying on standard BM25 and vector score blending without domain tuning.

**Outcome:** Benchmark LanceDB hybrid retrieval with tuned lexical/vector weighting, score filtering thresholds, and acronym expansions to maximize citation precision.

**Acceptance:** Measured retrieval precision and recall on technical evaluation documents improves over the baseline without exceeding chunk retrieval latency budgets.

**References:** [DEV-11](development.md), [decisions.md](decisions.md).
