---
title: Open-source readiness audit
normative-for: [public-source release gate, future binary-release requirements]
depends: [tracker.md, third-party-license-inventory.json, app-architecture.md, data-schema.md]
status: current release gate. First-party source code released under MIT.
---

# Open-source readiness audit

**Assessment date:** 2026-11-26

The current publication target is an experimental, source-only GitHub repository at `MRKups/ragtag`, with `main` as its default branch. No installer or binary release is planned. This checklist is not legal advice or a license selection.

## Licensing resolution

First-party source code is released under the [MIT License](../LICENSE) by copyright holder Michal Kups. Package manifests (`package.json`, `frontend/package.json`, `src-tauri/Cargo.toml`, and crate manifests) declare `MIT` licensing and repository metadata.

The vendored `mistralrs-core` retains its upstream MIT `LICENSE` and package metadata. Treat this local-path dependency as third-party material when assembling provenance and notices; [D007](decisions.md) records Ragtag's source patch.

## Source-publication checks

| Gate | Current state | Required evidence |
| --- | --- | --- |
| First-party rights and license | Resolved: [LICENSE](../LICENSE) | Selected MIT license, aligned repository and package metadata. |
| PDF parser allocation limit | Resolved: [D008](decisions.md) | Streaming parser bounds without artificial decompression budget. |
| External inference transport | Resolved: [D015](decisions.md) | Keyless probing, credential fallback, and HTTP/HTTPS provider support. |
| Default model artifact integrity | Implemented: [D011](decisions.md) | SHA-256 cache-identity checks for fresh and cached GGUF files; this does not authenticate publishers or verify all runtime assets. |
| Local inference upgrade qualification | Open: [B189](tracker.md) | Native tests, real-engine smoke checks, and source-built desktop behavior on Apple Silicon. |
| Source licensing and dependency inventory | Implemented | Retained upstream licenses, current lockfile inventory, and passing source-release metadata checks. This is not a final-installer SBOM. |
| Model terms | Documented: [default runtime assets](../THIRD_PARTY_NOTICES.md#machine-learning-models--weights-notice) | Gemma terms for the default embedder and Apache-2.0 for Qwen3 weights/tokenizer assets, separate from first-party MIT. Runtime downloads are not included in source distribution. |
| Source provenance | Reviewed on 2026-11-26 | Aligned versions, reviewed upstream revision and local patch, and no confirmed private documents or credentials in the source/history scans. Signing or notarizing an app is not required for source publication. |
| Public-project operations | Open | A security-reporting route and a stated support/contribution policy. GitHub review rules and automation can be added without distributing binaries. |
| README screenshots | Implemented: [README gallery](../README.md#screenshots) | Four user-supplied screenshots reviewed for visible secrets and personal paths; published copies contain only WebP image data. |

## Dependency boundary

[`third-party-license-inventory.json`](third-party-license-inventory.json) contains raw license declarations for the default-feature Cargo resolution and frontend lockfile, not final-installer content. Cargo packages reachable only through disabled optional features, such as CUDA, may be absent and require separate review if those features are enabled.

Review the vendored source and retained notices before source publication. A future binary release would separately inspect bundled JavaScript, CSS, SVG, native libraries, generated assets, runtime artifacts, and distributed models. Lockfile declarations are not legal approval or notice compliance.

## Data and external-service boundary

Ragtag keeps sources, indexes, models, and default inference local. Optional external inference sends the query and retrieved excerpts to the selected endpoint over HTTP or HTTPS as configured; current transport and credential handling behavior is in [Application architecture](app-architecture.md), [Data schema and IPC contracts](data-schema.md), and [Design rationale](decisions.md).

## Future binary distribution — not a source-publication gate

If binary distribution is introduced later, retain:

1. A final-installer SPDX or CycloneDX SBOM.
2. Human-readable third-party notices with required license text and attribution.
3. Model provenance and notice data for every distributed or default-downloaded model.
4. Signed archive and SHA-256 checksum.
5. macOS signing, notarization, and stapling evidence where applicable.
6. The release version, source revision, and provenance attestation.

## Legal and provenance status

First-party copyright ownership and MIT licensing are established. Model weights retrieved by users remain subject to their respective upstream licenses. Declared third-party dependency licenses within the scope above are recorded in `third-party-license-inventory.json`; this is not legal approval.
