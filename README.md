# Ragtag

Ragtag is a local-first desktop RAG application for querying user-provided PDFs, DOCX files, and UTF-8 technical documents. It copies documents into app-local storage, builds local search indexes, and answers questions with retrieved source citations. By default, document processing, embeddings, vector search, and generation run on the user's machine; an external OpenAI-compatible inference endpoint is an optional user-selected mode.

## Prerequisites

Ragtag is configured for macOS development and Apple Silicon distribution. Before the first native build, install:

- **Node.js 24.20.0 LTS** and npm. The repository root `.nvmrc` and package `engines` range enforce Node 24:

  ```sh
  nvm install
  nvm use
  ```

- **Rust 1.98.0**, installed through [rustup](https://rustup.rs/). `rust-toolchain.toml` selects this version because the current LanceDB and SQLx releases require Rust 1.91 and 1.94 respectively.
- **Xcode Command Line Tools** for macOS builds. Apple Silicon builds use Metal for local generation; Intel development builds use CPU inference.
- **Protocol Buffers** for LanceDB's generated protocol types:

  ```sh
  brew install protobuf
  ```

- **Tauri CLI 2.11.4**:

  ```sh
  cargo install tauri-cli --version 2.11.4 --locked
  ```

- **go-task** for the repository build tasks:

  ```sh
  brew install go-task
  ```

Install the Metal Toolchain for Apple Silicon development if Xcode does not already provide it:

```sh
sudo xcodebuild -downloadComponent MetalToolchain
```

Confirm the required tools are available:

```sh
node --version
npm --version
rustc --version
cargo --version
protoc --version
cargo tauri --version
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

- copied uploaded source documents;
- the `app.db` SQLite document registry and non-secret settings;
- the `.lancedb` vector store containing derived document chunks and embeddings;
- downloaded embedding and local-generation model files.

The first local launch downloads the model artifacts needed for offline use: approximately 800 MB for EmbeddingGemma-300M embeddings and approximately 2 GB for the default Qwen3-1.7B GGUF generator. Downloads resume after interruption and report progress in the app boot overlay.

The **Settings** page can select external inference instead. It stores the endpoint URL as a local setting and stores the API key in the operating system keychain, never in SQLite or frontend storage. External inference skips the local generation-model download, but first verifies the configured/default model through the endpoint’s OpenAI-compatible `/models` route before enabling queries. Document ingestion and retrieval remain local.

## Documents and citations

Ragtag accepts PDF and DOCX documents plus UTF-8 text sources such as Markdown, configuration, structured data, logs, and source files. PDF ingestion preserves physical page boundaries; DOCX and non-paginated text use one logical source page. Citation previews render PDFs with PDF.js; indexed UTF-8 and DOCX previews are processed locally in Rust, with generic text shown verbatim and Markdown rendered from a safe typed model that excludes raw HTML, embeds, images, and unsafe links. Ingestion extracts text, creates overlapping chunks, and writes 768-dimensional embeddings to the local LanceDB store. Query results combine dense vector and full-text retrieval; the response cites retrieved source metadata rather than model-generated page numbers.

Deleting a document removes its derived vectors, registry entry, and stored copy. The **Settings** page also exposes maintenance actions for SQLite vacuuming, vector-store compaction, and an explicitly confirmed reset of local data.

## Development

### Apple Silicon native app with live reload

```sh
task dev
```

Use this for normal Apple Silicon development. It runs `cargo tauri dev`, starts the Vite development server at `http://localhost:5173`, compiles the Rust workspace in debug mode, and opens the native Ragtag window.

Frontend Svelte, TypeScript, and CSS changes update through Vite hot-module reload. Rust, Tauri configuration, capability, model, and persistence changes require the native process to rebuild or restart. Stop the development stack with `Ctrl-C`.

### Intel macOS development only

Ragtag releases support Apple Silicon only. An Intel Mac may still compile and run the development build by linking the current FastEmbed/ORT stack against an **external**, x86_64 ONNX Runtime 1.28.0 shared library. Do not add this dylib, its build output, environment variables, entitlements, or an Intel target to the repository or release bundle.

The current ONNX Runtime release does not provide a prebuilt Intel macOS archive. Build the matching runtime outside the checkout once:

```sh
ORT_ROOT="$HOME/.cache/mrk-ragtag/onnxruntime-1.28.0"
git clone --depth 1 --branch v1.28.0 https://github.com/microsoft/onnxruntime.git "$ORT_ROOT"
cd "$ORT_ROOT"
./build.sh --config Release --build_shared_lib --parallel --skip_tests \
  --cmake_extra_defines CMAKE_DISABLE_FIND_PACKAGE_Protobuf=TRUE

```

This CMake definition deliberately builds ONNX Runtime's pinned Protobuf source instead of mixing its bundled protoc 21.12 with Homebrew's newer Protobuf headers. It is required when `brew install protobuf` is present for LanceDB.

Use the Intel tasks after the one-time runtime build:

```sh
task dev-intel
# Or compile the native debug application without launching it:
task build-intel
```

The tasks validate the external x86_64 dylib, set `ORT_LIB_LOCATION`, dynamic-linking, and runtime rpath environment variables, then launch or compile the application. The default path is `$HOME/.cache/mrk-ragtag/onnxruntime-1.28.0`; set `RAGTAG_INTEL_ORT_ROOT` to use another external location. If Cargo previously built `ort-sys` without these variables, run `cargo clean --manifest-path src-tauri/Cargo.toml -p ort-sys` once before retrying.

The local LLM runs on CPU in Intel development builds; Metal remains enabled only for Apple Silicon. This is development-only and does not add Intel release support.

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
