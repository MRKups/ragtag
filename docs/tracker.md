---
title: Ragtag implementation tracker
normative-for: [unresolved implementation work]
depends: [app-architecture.md, data-schema.md, decisions.md, open-source-readiness.md]
status: current work only. Completed and superseded items are removed.
---

# Ragtag implementation tracker

Items state unresolved outcomes. Current behavior belongs in linked references.

## B189 — Qualify the pinned engine on Apple Silicon

**Problem:** Packaged Apple Silicon application verification requires end-to-end qualification of Metal execution.

**Outcome:** The pinned inference stack is exercised on an Apple Silicon host using the existing native test, release-build, and bundle tasks.

**Acceptance:** Launch the packaged app and verify actual Metal model loading, streamed completion with citations, cancellation and subsequent queries, concurrent queries, cached offline restart, and provider switching. Record startup, first-token latency, generation performance, and peak memory against a same-model baseline; review material regressions before accepting them.

**References:** [D007 and D013](decisions.md), [README build instructions](../README.md), [release gate](open-source-readiness.md).
