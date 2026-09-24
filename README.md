# Ragtag

Ragtag is a local-first macOS desktop application for querying user-provided PDFs, DOCX files, and UTF-8 text documents with retrieved-page citations. Default document processing, embeddings, vector search, and generation run on the device. External OpenAI-compatible inference is optional.

> **Project status:** Ragtag is private and not ready for public source publication or public binary release. See the [open-source readiness audit](docs/open-source-readiness.md).

## Prerequisites

Development requires macOS, Node.js 24, Rust 1.98, Xcode Command Line Tools, Protocol Buffers, the Tauri CLI v2, and [go-task](https://taskfile.dev/).

```sh
nvm install
nvm use
brew install protobuf go-task
cargo install tauri-cli --locked
```

On Apple Silicon, install the Metal toolchain if Xcode does not provide it:

```sh
sudo xcodebuild -downloadComponent MetalToolchain
```

Check the toolchain before building:

```sh
node --version
npm --version
rustc --version
cargo --version
protoc --version
cargo tauri --version
task --version
```

## Install and run

From the repository root:

```sh
task setup
task dev
```

`task setup` installs frontend dependencies and verifies Rust. `task dev` starts Vite and the native application with live reload.

For frontend-only work:

```sh
npm run dev --prefix frontend
```

This starts Vite without Tauri IPC, keychain access, native filesystem behavior, or the asset protocol.

## Build profile and upstream deviations

Ragtag does not use a stock crates.io-only inference build. Its generation SDK is pinned to the `mistral.rs` **v0.9.3 Git revision**, while that release also pins Candle to Git. Cargo resolves those sources through the application lockfile; installing a newer standalone `mistralrs` executable does not change Ragtag's embedded engine.

The checked-in `src-tauri/vendor/mistralrs-core` overrides **only the core package** through Cargo's patch mechanism. The unmodified release fails some concurrent GGUF queries because batched output selection produces strided tensors that quantized projection cannot consume. Ragtag's local patch makes those selected rows contiguous, rather than disabling concurrency or serializing requests. The vendor manifest resolves upstream workspace inheritance, keeps sibling crates Git-pinned, and retains the upstream MIT license. Runtime revision reporting identifies the local patch instead of incorrectly reporting Ragtag's own Git revision.

A working development environment needs three independent pieces:

- **Build inputs:** the pinned source graph, checked-in vendor sources and test fixtures, compatible Rust/native toolchains, and any platform libraries. An offline Rust build additionally requires the resolved Cargo sources to be cached.
- **Inference inputs:** compatible model and tokenizer assets in their expected caches, not merely a successfully linked executable. First-time artifact downloads and engine compilation are separate concerns. The engine can select a GGUF's embedded tokenizer when external tokenizer metadata is incompatible.
- **Platform qualification:** Ragtag targets Apple Silicon macOS with Metal acceleration and requires its own runtime and packaged-app verification.

The native test tasks run the application workspace suite and the focused vendored batching regression. The vendor package is deliberately outside the first-party workspace; this is not a claim that the entire upstream test suite or every supported model/backend has been qualified. Current outstanding acceptance gates are recorded in [the tracker](docs/tracker.md).

Maintain the vendor tree as an explicit, minimal deviation from its recorded upstream revision, not as a second independently evolving engine. Replacing the base revision or retiring the override requires comparing upstream changes, preserving equivalent regression coverage, refreshing dependency provenance, and rechecking real model loading, streaming, cancellation, concurrency, and the supported release platform. Keep the override until those checks demonstrate that upstream supplies the repair. [D007](docs/decisions.md#d007--local-in-process-inference-by-default) owns the exact source and patch rationale.

## Local data and inference

Ragtag stores managed document copies, the SQLite registry, the LanceDB vector store, and app-managed model files in application-local data. The optional external-provider API key stays in the OS keychain.

Ragtag accepts PDF, DOCX, and UTF-8 text. PDFs retain physical page identity. DOCX and text use one logical source page for citations.

The first local use downloads model artifacts. An external provider receives the query and retrieved excerpts during generation. Use only a provider and data-handling policy you trust. [B186](docs/tracker.md) remains an open release gate.

## Checks and tests

```sh
task check
npm test
```

`task check` runs Svelte and TypeScript checks. `npm test` runs frontend state and error tests, then Rust workspace tests and the focused vendored-core regression.

## Builds

```sh
task build-frontend
task build
task bundle
```

`task build-frontend` creates `frontend/dist`. `task build` creates `src-tauri/target/release/mrk-ragtag-app`. `task bundle` creates `src-tauri/target/release/bundle/macos/Ragtag.app`.

These commands do not create a signed or publicly releasable distribution. See the [open-source readiness audit](docs/open-source-readiness.md).

## Cleanup

```sh
task clean-frontend
task clean-rust
task clean
```

These tasks remove build output, not application-local documents, databases, vectors, models, or keychain credentials.

## Documentation

- [Documentation index](docs/README.md)
- [Application architecture](docs/app-architecture.md)
- [Data schema and IPC contracts](docs/data-schema.md)
- [Design rationale](docs/decisions.md)
- [File structure](docs/file-structure.md)
- [Implementation tracker](docs/tracker.md)
