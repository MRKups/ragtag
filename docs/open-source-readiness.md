---
title: Open-source readiness audit
normative-for: [public-source and public-binary release gate]
depends: [tracker.md, third-party-license-inventory.json, app-architecture.md, data-schema.md]
status: current release gate. First-party source code released under MIT.
---

# Open-source readiness audit

**Assessment date:** 2026-09-25

This is the current release gate, not legal advice or a license selection.

## Licensing resolution

First-party source code is released under the [MIT License](../LICENSE) by copyright holder Michal Kups. Package manifests (`package.json`, `frontend/package.json`, `src-tauri/Cargo.toml`, and crate manifests) declare `MIT` licensing and repository metadata.

The vendored `mistralrs-core` retains its upstream MIT `LICENSE` and package metadata. Treat this local-path dependency as third-party material when assembling provenance and notices; [D007](decisions.md) records Ragtag's source patch.

## Release gates

| Gate | Current state | Required evidence |
| --- | --- | --- |
| First-party rights and license | Resolved: [LICENSE](../LICENSE) | Selected MIT license, aligned repository and package metadata. |
| PDF parser allocation limit | Resolved: [D008](decisions.md) | Streaming parser bounds without artificial decompression budget. |
| External inference transport | Resolved: [D015](decisions.md) | Keyless probing, credential fallback, and HTTP/HTTPS provider support. |
| Default model artifact integrity | Resolved: [D011](decisions.md) | Configurable Hugging Face repositories and GGUF filenames with lightweight defaults. |
| Local inference upgrade qualification | Open: [B189](tracker.md) | Observed Apple Silicon Metal and packaged-app behavior. |
| Dependency and native-component closure | Open | Final-installer SBOM and third-party notices that cover shipped native code and emitted frontend assets. |
| Model and platform terms | Open | Current evidence for every distributed or default-downloaded model and every bundled platform component. |
| Release provenance | Open | Version alignment, signed bundle, checksum, signing and notarization evidence, and release provenance. |
| Public-project operations | Open | Security reporting, contribution policy, code of conduct, review policy, and issue or pull-request process. |

## Dependency boundary

[`third-party-license-inventory.json`](third-party-license-inventory.json) contains declared Rust and npm lockfile facts, not final-installer content.

Release closure separately inspects bundled JavaScript, CSS, SVG, native libraries, generated assets, runtime artifacts, and distributed models. Lockfile declarations are not legal approval or notice compliance.

## Data and external-service boundary

Ragtag keeps sources, indexes, models, and default inference local. Optional external inference sends the query and retrieved excerpts to the selected endpoint over HTTP or HTTPS as configured; current transport and credential handling behavior is in [Application architecture](app-architecture.md), [Data schema and IPC contracts](data-schema.md), and [Design rationale](decisions.md).

## Required release evidence

Public releases retain:

1. A final-installer SPDX or CycloneDX SBOM.
2. Human-readable third-party notices with required license text and attribution.
3. Model provenance and notice data for every distributed or default-downloaded model.
4. Signed archive and SHA-256 checksum.
5. macOS signing, notarization, and stapling evidence where applicable.
6. The release version, source revision, and provenance attestation.

## Legal and provenance status

First-party copyright ownership and MIT licensing are established. Model weights retrieved by users remain subject to their respective upstream licenses. Full third-party dependency licensing is inventoried in `third-party-license-inventory.json`.
