---
title: Ragtag implementation tracker
normative-for: [unresolved implementation work]
depends: [app-architecture.md, data-schema.md, decisions.md, open-source-readiness.md]
status: current work only. Completed and superseded items are removed.
---

# Ragtag implementation tracker

Each item states the unresolved outcome. Current behavior belongs in the linked reference documents. Historical findings belong in [`archive/reviews/`](archive/reviews/).

## B185 — Bound PDF decompression before parser allocation

**Problem:** Current source, page, and retained-text limits do not bound allocation while the PDF parser inflates compressed content or object streams.

**Outcome:** The extraction path enforces a cumulative decoder allocation budget before unbounded parser allocation.

**Acceptance:** Controlled tests cover compressed streams, object streams, and PNG predictors. Each path rejects input before exceeding the configured budget. Documentation describes the limit only after end-to-end verification.

**References:** [Architecture](app-architecture.md), [D008](decisions.md), [release gate](open-source-readiness.md).

## B186 — Restrict external inference transport

**Problem:** The configured external-provider path accepts HTTP endpoints and can send bearer credentials, queries, and retrieved excerpts to them.

**Outcome:** Remote endpoints require HTTPS. Any retained plaintext local-provider exception is verified as loopback-only, visibly warned, and unavailable to non-loopback addresses.

**Acceptance:** Persisted insecure configurations are rejected before a model probe or chat request. Boundary tests cover HTTPS, IPv4 and IPv6 loopback, `localhost`, LAN and public addresses, redirects, and malformed authorities.

**References:** [Architecture](app-architecture.md), [data and IPC contract](data-schema.md), [D007](decisions.md), [release gate](open-source-readiness.md).

## B187 — Pin and verify default model artifacts

**Problem:** Default GGUF downloads identify a mutable Hugging Face repository and filename without an immutable revision, expected size, or digest.

**Outcome:** A versioned default-model manifest names the publisher, repository, immutable revision, filename, size, SHA-256, source, and terms. Downloads are verified before model construction.

**Acceptance:** Tests cover valid artifacts, revision mismatch, checksum mismatch, interrupted downloads, stale caches, and custom sources. User-selected repositories remain explicitly unverified trust boundaries.

**References:** [Architecture](app-architecture.md), [D011](decisions.md), [release gate](open-source-readiness.md).
