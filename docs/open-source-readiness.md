---
title: Open-source readiness audit
normative-for: [public-source and public-binary release gate]
depends: [tracker.md, third-party-license-inventory.json, app-architecture.md, data-schema.md]
status: current release gate. Not ready for public source publication or public binary release.
---

# Open-source readiness audit

**Assessment date:** 2026-09-16

This is the current release gate, not legal advice or a license selection.

## Current blocker

The root `LICENSE` reserves all rights and grants no permission to use, copy, modify, or distribute. Root `package.json` declares `UNLICENSED`. The frontend and Rust workspace package manifests do not declare a first-party license.

Public source publication and public binary distribution remain blocked until the rights holder confirms ownership of all first-party material, selects a license, replaces the pending license text, and aligns first-party package metadata.

The vendored `mistralrs-core` retains its upstream MIT `LICENSE` and package metadata. Treat this local-path dependency as third-party material when assembling provenance and notices; [D007](decisions.md) records Ragtag's source patch.

## Release gates

| Gate | Current state | Required evidence |
| --- | --- | --- |
| First-party rights and license | Open | Ownership review, selected license, aligned repository and package metadata, and contribution terms. |
| PDF parser allocation limit | Open: [B185](tracker.md) | End-to-end bounded extraction tests for compressed content. |
| External inference transport | Open: [B186](tracker.md) | Enforced remote HTTPS policy, any verified loopback exception, and boundary tests. |
| Default model artifact integrity | Open: [B187](tracker.md) | Immutable model manifest and verified download behavior. |
| Local inference upgrade qualification | Open: [B189](tracker.md) | Observed Apple Silicon Metal and packaged-app behavior. |
| Dependency and native-component closure | Open | Final-installer SBOM and third-party notices that cover shipped native code and emitted frontend assets. |
| Model and platform terms | Open | Current evidence for every distributed or default-downloaded model and every bundled platform component. |
| Release provenance | Open | Version alignment, signed bundle, checksum, signing and notarization evidence, and release provenance. |
| Public-project operations | Open | Security reporting, contribution policy, code of conduct, review policy, and issue or pull-request process. |

## Dependency boundary

[`third-party-license-inventory.json`](third-party-license-inventory.json) contains declared Rust and npm lockfile facts, not final-installer content.

Release closure separately inspects bundled JavaScript, CSS, SVG, native libraries, generated assets, runtime artifacts, and distributed models. Lockfile declarations are not legal approval or notice compliance.

## Data and external-service boundary

Ragtag keeps sources, indexes, models, and default inference local. Optional external inference sends the query and retrieved excerpts to the selected endpoint. [B186](tracker.md) must close before release; current transport behavior is in [Application architecture](app-architecture.md) and [Data schema and IPC contracts](data-schema.md).

## Required release evidence

Public releases retain:

1. A final-installer SPDX or CycloneDX SBOM.
2. Human-readable third-party notices with required license text and attribution.
3. Model provenance and notice data for every distributed or default-downloaded model.
4. Signed archive and SHA-256 checksum.
5. macOS signing, notarization, and stapling evidence where applicable.
6. The release version, source revision, and provenance attestation.

## Unresolved legal and provenance questions

The repository does not establish final rights ownership, license selection, contribution terms, branding ownership, or model and platform term sufficiency. Resolve them with owner and legal review before presenting Ragtag as open-source ready.
