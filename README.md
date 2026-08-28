# Ragtag

Ragtag is a local-first desktop RAG application for querying user-provided PDFs and technical manuals. It copies documents into app-local storage, builds local search indexes, and answers questions with retrieved page citations. By default, document processing, embeddings, vector search, and generation run on the user's machine; an external OpenAI-compatible inference endpoint is an optional user-selected mode.

## Prerequisites

Ragtag is configured for macOS development and distribution. Before the first native build, install:

- **Node.js 20+** and npm.
- **Rust 2024 edition**, installed through [rustup](https://rustup.rs/).
- **Xcode** and the Metal Toolchain. The macOS build enables `mistralrs` Metal support for local generation.
- **Tauri CLI v2**:

  ```sh
  cargo install tauri-cli --version "^2"
  ```

- **go-task** for the repository build tasks:

  ```sh
  brew install go-task
  ```

Install the Metal Toolchain if Xcode does not already provide it:

```sh
sudo xcodebuild -downloadComponent MetalToolchain
```

Confirm the required tools are available:

```sh
node --version
npm --version
rustc --version
cargo --version
task --version
xcrun metal --version
```

If Metal shader precompilation is unavailable, set `MISTRALRS_METAL_PRECOMPILE=0` for the Cargo or Tauri build command. This defers shader compilation to first model load.

## Install dependencies

Run this from the repository root after cloning, or after deliberately removing frontend dependencies:

```sh
task setup
```

`task setup` runs `npm install` in `frontend/` and verifies that `cargo` and `rustc` are available. The frontend package owns Svelte, Vite, TypeScript, and the frontend Tauri API; Rust dependencies are managed by the workspace in `src-tauri/Cargo.toml`.

Use `npm install` only when intentionally changing frontend dependencies. This repository's Taskfile is the supported entry point for application setup, development, checks, and builds.

## Local data and inference

Ragtag requires no local `.env` file or cloud account for its default mode. It stores the following in the operating system's application-local data directory:

- copied uploaded PDFs;
- the `app.db` SQLite document registry and non-secret settings;
- the `.lancedb` vector store containing derived document chunks and embeddings;
- downloaded embedding and local-generation model files.

The first local launch downloads the model artifacts needed for offline use: approximately 800 MB for EmbeddingGemma-300M embeddings and approximately 2 GB for the default Qwen3-1.7B GGUF generator. Downloads resume after interruption and report progress in the app boot overlay.

The **Settings** page can select external inference instead. It stores the endpoint URL as a local setting and stores the API key in the operating system keychain, never in SQLite or frontend storage. External inference skips the local generation-model download, but document ingestion and retrieval remain local.

## Documents and citations

Ragtag currently accepts PDF documents. Ingestion preserves page boundaries, extracts text, creates overlapping chunks, and writes 768-dimensional embeddings to the local LanceDB store. Query results combine dense vector and full-text retrieval; the response cites the retrieved document/page metadata rather than model-generated page numbers.

Deleting a document removes its derived vectors, registry entry, and stored copy. The **Settings** page also exposes maintenance actions for SQLite vacuuming, vector-store compaction, and an explicitly confirmed reset of local data.

## Development

### Full native app with live reload

```sh
task dev
```

Use this for normal Ragtag development. It runs `cargo tauri dev`, starts the Vite development server at `http://localhost:5173`, compiles the Rust workspace in debug mode, and opens the native Ragtag window.

Frontend Svelte, TypeScript, and CSS changes update through Vite hot-module reload. Rust, Tauri configuration, capability, model, and persistence changes require the native process to rebuild or restart. Stop the development stack with `Ctrl-C`.

### Frontend only

```sh
npm run dev --prefix frontend
```

Use this for Svelte markup, styling, or browser-oriented UI work. It starts Vite at `http://localhost:5173`, but does not compile Rust or open a native window. Tauri IPC, filesystem access, keychain calls, and the local PDF asset protocol are unavailable in this mode.

## Checks and tests

### Frontend type and Svelte checks

```sh
task check
```

Run this after changing Svelte, TypeScript, or Vite configuration. It runs `svelte-check` against `frontend/`.

### Rust workspace checks

The Taskfile does not define a Rust test task. For a Rust-only compilation or test pass, run the narrowest applicable Cargo command from the workspace manifest, for example:

```sh
cargo test --manifest-path src-tauri/Cargo.toml -p domain
cargo test --manifest-path src-tauri/Cargo.toml -p persistence
cargo test --manifest-path src-tauri/Cargo.toml -p rag_pipeline
```

Use a full workspace test command only when changes span multiple crates or application composition:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

## Builds

### Frontend production output only

```sh
task build-frontend
```

Use this to validate the Vite production build or regenerate `frontend/dist/` without compiling Rust. Tauri bundles these files for native builds.

### Release binary

```sh
task build
```

This runs the frontend checks and production build, then creates the optimized unbundled binary:

```text
src-tauri/target/release/mrk-ragtag-app
```

### macOS application bundle

```sh
task bundle
```

This runs the frontend checks and production build, then produces the distributable macOS application bundle:

```text
src-tauri/target/release/bundle/macos/Ragtag.app
```

The bundle includes the native application and frontend assets. Local model artifacts are intentionally downloaded on first run rather than embedded in the application bundle.

## Cleanup

### Remove frontend build output

```sh
rm -rf frontend/dist
```

Safe to run whenever a clean Vite output is needed; the next frontend or Tauri build recreates it.

### Remove Rust artifacts and Tauri bundles

```sh
task clean-rust
```

This runs `cargo clean` from `src-tauri/` and removes `src-tauri/target/`, including compiled dependencies, release binaries, and native bundles. The next native build will take longer because Cargo must compile them again.

### Remove all generated build output

```sh
task clean
```

This removes frontend build output, the Vite cache, and Rust build artifacts. It does not remove application-local documents, databases, vector indexes, model files, or OS-keychain credentials.

Do not use `git clean -fdX` as a general cleanup command. Ignored files can include local build state and developer configuration that you intend to keep.

## Project layout

See [the repository layout](docs/file-structure.md) for code ownership and entry points. Read [the application architecture](docs/app-architecture.md) for runtime boundaries and data flows, [the data schema](docs/data-schema.md) for local persistence and IPC contracts, and [the design decisions](docs/decisions.md) for the rationale behind the local-first stack.
