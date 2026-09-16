# Ragtag

Ragtag is a local-first macOS desktop application for querying user-provided PDFs, DOCX files, and UTF-8 text documents. It returns answers with citations from retrieved source pages.

Document processing, embeddings, vector search, and default generation run on the device. External OpenAI-compatible inference is an explicit user-selected alternative.

> **Project status:** Ragtag is private and not ready for public source publication or public binary release. See the [open-source readiness audit](docs/open-source-readiness.md).

## Prerequisites

Development requires macOS, Node.js 24, Rust 1.98, Xcode Command Line Tools, Protocol Buffers, the Tauri CLI v2, and [go-task](https://taskfile.dev/).

```sh
nvm install
nvm use
brew install protobuf go-task
cargo install tauri-cli --locked
```

Install the Metal toolchain on Apple Silicon if Xcode does not already provide it:

```sh
sudo xcodebuild -downloadComponent MetalToolchain
```

Confirm the toolchain before building:

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

`task setup` installs frontend dependencies and verifies the Rust toolchain. `task dev` starts the Vite development server and the native Tauri application with live reload.

For frontend-only work:

```sh
npm run dev --prefix frontend
```

This starts Vite but does not provide Tauri IPC, keychain access, native filesystem behavior, or the Tauri asset protocol.

## Intel macOS development

Ragtag releases target Apple Silicon. Intel macOS remains a development and test path that requires an external x86_64 ONNX Runtime 1.28.0 build outside the checkout.

Build the matching runtime once if it is not already available:

```sh
ORT_VERSION=1.28.0
ORT_ROOT="$HOME/.cache/mrk-ragtag/onnxruntime-$ORT_VERSION"
git clone --depth 1 --branch "v$ORT_VERSION" https://github.com/microsoft/onnxruntime.git "$ORT_ROOT"
cd "$ORT_ROOT"
./build.sh --config Release --build_shared_lib --parallel --skip_tests \
  --cmake_extra_defines CMAKE_DISABLE_FIND_PACKAGE_Protobuf=TRUE
```

The build must produce `build/MacOS/Release/libonnxruntime.1.28.0.dylib`. Use the Intel tasks after that:

```sh
task dev-intel
task build-intel
task test-intel
```

Set `RAGTAG_INTEL_ORT_ROOT` when the runtime lives outside the default cache path. The runtime and its build output must not enter the repository or release bundle.

## Local data and inference

Ragtag stores managed document copies, the SQLite registry, the LanceDB vector store, and app-managed model files in the operating system application-local data directory. The optional external-provider API key is stored only in the OS keychain.

Ragtag accepts PDF, DOCX, and UTF-8 text sources. PDFs preserve physical page identity. DOCX and UTF-8 sources use one logical source page for citations.

The first local use downloads required model artifacts. The Settings page can instead select an external OpenAI-compatible provider. That provider receives the query and retrieved excerpts during generation. Configure only a provider and data-handling policy you trust. The current remote-transport restriction remains an open release gate in [B186](docs/tracker.md).

## Checks and tests

```sh
task check
npm test
```

`task check` runs Svelte and TypeScript checks. `npm test` runs maintained frontend state and error tests, then the Rust workspace tests for the active development architecture.

## Builds

```sh
task build-frontend
task build
task bundle
```

`task build-frontend` creates `frontend/dist`. `task build` produces an unbundled native binary at `src-tauri/target/release/mrk-ragtag-app`. `task bundle` produces a local macOS application bundle at `src-tauri/target/release/bundle/macos/Ragtag.app`.

These commands do not create a signed or publicly releasable distribution. Public release requirements are recorded in the [open-source readiness audit](docs/open-source-readiness.md).

## Cleanup

```sh
task clean-frontend
task clean-rust
task clean
```

These tasks remove build output. They do not remove application-local documents, databases, vectors, models, or keychain credentials.

## Documentation

- [Documentation index](docs/README.md)
- [Application architecture](docs/app-architecture.md)
- [Data schema and IPC contracts](docs/data-schema.md)
- [Design rationale](docs/decisions.md)
- [File structure](docs/file-structure.md)
- [Implementation tracker](docs/tracker.md)
