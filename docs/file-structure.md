---
title: File structure
normative-for: [repository navigation, package ownership, dependency direction]
depends: [app-architecture.md, data-schema.md]
status: current guide. Source paths override this navigation map.
---

# File structure

Use this document to locate a subsystem. It does not define runtime behavior.

## Root

- `README.md`: User-facing setup, operation, development, checks, and build guide.
- `Taskfile.yml`: Development, checking, build, bundle, and cleanup tasks.
- `docs/`: Current guides, specifications, and records. Start at [`docs/README.md`](README.md).
- `frontend/`: Svelte 5 and Vite interface.
- `src-tauri/`: Tauri application and Rust workspace.

## `frontend/`

- `index.html`: Vite HTML entry point.
- `package.json` and `package-lock.json`: Frontend runtime, development dependencies, scripts, and locked resolution.
- `src/main.ts`: Interface entry point.
- `src/App.svelte`: Application shell and top-level navigation.
- `src/pages/`: Query, document-management, and settings pages.
- `src/lib/api.ts`: The sole frontend import boundary for Tauri APIs and typed IPC wrappers.
- `src/lib/types.ts`: Frontend representations of IPC payloads.
- `src/lib/components/`: Reusable interface components.
- `src/lib/stores/`: Svelte rune stores for application, query, settings, document-progress, and error-dialog state.
- `src/lib/assets/`: Bundled frontend assets.
- `src/lib/*.test.ts` and `src/lib/stores/*.test.ts`: Maintained frontend error and state tests.

The frontend is a client-only hash-routed SPA with Tauri access only through `src/lib/api.ts`.

## `src-tauri/`

- `Cargo.toml` and `Cargo.lock`: Rust workspace manifest and locked dependency resolution.
- `tauri.conf.json`: Tauri application and bundle configuration.
- `capabilities/`: Tauri capability declarations.
- `icons/`: Application icons.
- `src/main.rs`: Native binary entry point.
- `src/lib.rs`: Application composition, builder setup, plugin registration, and background worker initialization.
- `src/state.rs`: Shared application state types.
- `src/model_download.rs`: Local model-download reporting.
- `src/commands/`: Tauri IPC command entry points (`models`, `documents`, `query`, `settings`, `keys`, `maintenance`).
- `src/inference/`: Model inference execution (`local` via mistral.rs, `external` via OpenAI-compatible HTTP streaming, and `sse` decoding).
- `src/model_lifecycle/`: Model download coordination, generation coalescing, and startup vector recovery.
- `src/storage/`: Managed source file validation, atomic writes, orphan reconciliation, and app data resets.
- `src/workers/`: Document ingestion queue worker, retry loops, and progress forwarders.
- `crates/`: Domain, persistence, and document-processing libraries.
- `.sqlx/`: Committed SQLx offline query metadata.
- `vendor/mistralrs-core/`: Licensed upstream v0.9.3 core with Ragtag's explicit tensor-selection repair. Cargo patches this package into the Git-pinned SDK; it is excluded from the first-party workspace. Native test tasks run its focused regression separately. [D007](decisions.md) owns the deviation and retirement policy.
- `vendor/toml-selectors/`: Two unchanged upstream test fixtures needed to compile the vendored core's test target. They come from the same revision and MIT-licensed source as `vendor/mistralrs-core/`.

### `src-tauri/crates/domain/`

Shared serializable models, typed errors, and IPC event values. This crate has no Tauri, SQLite, LanceDB, filesystem, HTTP-provider, or UI dependency.

### `src-tauri/crates/persistence/`

SQLite repositories, migrations, and embedded LanceDB storage. [`data-schema.md`](data-schema.md) owns the stored-data contract.

### `src-tauri/crates/rag_pipeline/`

PDF, DOCX, and UTF-8 extraction, chunking, embedding, indexing preparation, and typed source preview generation.

## Dependency direction

```text
frontend -> Tauri application -> domain, persistence, rag_pipeline
rag_pipeline -> domain, persistence
persistence -> domain
```

The Tauri application composes the libraries. `domain` remains free of adapter dependencies.
