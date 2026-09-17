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

## Intel macOS development

Intel macOS is a development and test path only. It requires an external x86_64 ONNX Runtime 1.28.0 build outside the checkout.

Build the runtime once:

```sh
ORT_VERSION=1.28.0
ORT_ROOT="$HOME/.cache/mrk-ragtag/onnxruntime-$ORT_VERSION"
git clone --depth 1 --branch "v$ORT_VERSION" https://github.com/microsoft/onnxruntime.git "$ORT_ROOT"
cd "$ORT_ROOT"
./build.sh --config Release --build_shared_lib --parallel --skip_tests \
  --cmake_extra_defines CMAKE_DISABLE_FIND_PACKAGE_Protobuf=TRUE
```

The build must produce `build/MacOS/Release/libonnxruntime.1.28.0.dylib`. Then run:

```sh
task dev-intel
task build-intel
task test-intel
```

Set `RAGTAG_INTEL_ORT_ROOT` when the runtime is elsewhere. Do not commit or bundle the runtime or its build output.

## Local data and inference

Ragtag stores managed document copies, the SQLite registry, the LanceDB vector store, and app-managed model files in application-local data. The optional external-provider API key stays in the OS keychain.

Ragtag accepts PDF, DOCX, and UTF-8 text. PDFs retain physical page identity. DOCX and text use one logical source page for citations.

The first local use downloads model artifacts. An external provider receives the query and retrieved excerpts during generation. Use only a provider and data-handling policy you trust. [B186](docs/tracker.md) remains an open release gate.

## Checks and tests

```sh
task check
npm test
```

`task check` runs Svelte and TypeScript checks. `npm test` runs frontend state and error tests, then Rust workspace tests for the active development architecture.

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
