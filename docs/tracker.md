---
title: Ragtag implementation tracker
normative-for: [unresolved implementation work]
depends: [app-architecture.md, data-schema.md, decisions.md, open-source-readiness.md]
status: current work only. Completed and superseded items are removed.
---

# Ragtag implementation tracker

Items state unresolved outcomes. Current behavior belongs in linked references.

## B185 — Bound PDF decompression before parser allocation

**Problem:** Current source, page, and retained-text limits do not bound allocation while the PDF parser inflates compressed content or object streams.

**Outcome:** Extraction enforces a cumulative decoder allocation budget before parser allocation.

**Acceptance:** Tests cover compressed streams, object streams, and PNG predictors, each rejecting input before the configured budget. Document the limit only after end-to-end verification.

**References:** [Architecture](app-architecture.md), [D008](decisions.md), [release gate](open-source-readiness.md).

## B186 — Restrict external inference transport

**Problem:** The configured external-provider path accepts HTTP endpoints and can send bearer credentials, queries, and retrieved excerpts to them.

**Outcome:** Remote endpoints require HTTPS. Any retained plaintext local-provider exception is verified loopback-only, visibly warned, and unavailable elsewhere.

**Acceptance:** Insecure settings are rejected before a model probe or chat request. Tests cover HTTPS, IPv4 and IPv6 loopback, `localhost`, LAN and public addresses, redirects, and malformed authorities.

**References:** [Architecture](app-architecture.md), [data and IPC contract](data-schema.md), [D007](decisions.md), [release gate](open-source-readiness.md).

## B187 — Pin and verify default model artifacts

**Problem:** Default GGUF downloads identify a mutable Hugging Face repository and filename without an immutable revision, expected size, or digest.

**Outcome:** A versioned default-model manifest names the publisher, repository, immutable revision, filename, size, SHA-256, source, and terms. Downloads are verified before model construction.

**Acceptance:** Tests cover valid artifacts, revision and checksum mismatches, interrupted downloads, stale caches, and custom sources. User-selected repositories remain explicitly unverified.

**References:** [Architecture](app-architecture.md), [D011](decisions.md), [release gate](open-source-readiness.md).

## B189 — Qualify the pinned engine on Apple Silicon

**Problem:** Intel builds and CPU inference do not verify Metal execution or the packaged Apple Silicon application.

**Outcome:** The pinned inference stack is exercised on an Apple Silicon host using the existing native test, release-build, and bundle tasks.

**Acceptance:** Launch the packaged app and verify actual Metal model loading, streamed completion with citations, cancellation and subsequent queries, concurrent queries, cached offline restart, and provider switching. Record startup, first-token latency, generation performance, and peak memory against a same-model baseline; review material regressions before accepting them.

**References:** [D007 and D013](decisions.md), [README build instructions](../README.md), [release gate](open-source-readiness.md).
