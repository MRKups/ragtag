# Ragtag: Local-First RAG Desktop App

## What This Is

A localized, zero-configuration desktop application for querying technical manuals and other documents. It acts as an intelligent, personalized brain for technical manuals, allowing users to ask questions in natural language and receive synthesized answers with precise page citations.

It is a standalone executable that processes PDFs, generates embeddings, stores data, and searches vectors entirely on the user's local machine, ensuring absolute data privacy, instantaneous file retrieval, and zero cloud hosting costs.

### Core Features

- Fully offline/local document processing and storage
- Natural language queries about your documents
- Fast, secure desktop UI powered by Tauri v2
- Instantaneous document viewing directly from the local disk
- Citation engine matching AI answers to exact PDF pages
- Background processing for document ingestion without blocking the UI
- **BYO Inference Mode:** Option to securely connect to external OpenAI-compatible endpoints instead of local models

## Tech Stack

**App Shell & UI:**
- **Framework:** Tauri v2
- **Frontend:** Svelte 5 (Vite) / TypeScript
- **UI & Styling:** Svelte 5 (Native component-scoped CSS, no Tailwind)

**Backend Engine:**
- **Language:** Rust (Edition 2024+)
- **Async Runtime:** Tokio
- **IPC:** Tauri Commands & Events (using Tauri v2 Channels for streaming)
- **Database:** SQLite (via `sqlx`)
- **Vector DB:** LanceDB (Embedded, native Rust vector database)
- **Local Embeddings:** `fastembed-rs` (Runs ONNX models natively in Rust)
- **Local LLM (Generation):** `mistral.rs` (In-process, pure-Rust inference engine using Qwen3-1.7B — no sidecar)
- **Model Downloads:** `hf-hub` (Downloads model files safely on first-run with resume support)

## Prerequisites

- **Rust** (edition 2024+, install via [rustup](https://rustup.rs))
- **Node.js** 20+ and npm
- **Xcode** (macOS only) — required for the Metal GPU acceleration layer used by `mistralrs`. Without Xcode, the build will fail with an obscure Metal compiler error.
- **Tauri CLI** — `cargo install tauri-cli --version "^2"`

### macOS: Installing the Metal Toolchain

`mistralrs` compiles GPU shaders at build time using Apple's `metal` compiler. This requires the Metal Toolchain component from Xcode:

```
sudo xcodebuild -downloadComponent MetalToolchain
```

If the command reports "No components found," the toolchain is already integrated into your Xcode installation and no further action is needed.

Verify with:
```
xcrun metal --version
```

#### Troubleshooting: Build Fails with Metal Compiler Error

If you cannot install the toolchain (e.g., no admin access), you can skip build-time shader precompilation by prefixing cargo commands with:

```
MISTRALRS_METAL_PRECOMPILE=0
```

This defers shader compilation to runtime (adds a few seconds on first model load). Example:

```bash
MISTRALRS_METAL_PRECOMPILE=0 cargo build
```

Precompiled shaders are stored in the platform-agnostic `.metallib` format — the Metal driver handles final GPU-specific compilation at runtime, so shaders built on one macOS version are portable to others.

## How to Run

### First-Time Setup

```bash
npm install --prefix frontend
```

The first launch of the app will download model weights (~2.8GB total):
- Qwen3-1.7B GGUF (~2GB, for local LLM generation)
- Gemma 300M ONNX (~800MB, for embeddings)

Downloads are one-time and resume if interrupted. Progress is shown in the app's boot overlay.

### Development Build

```bash
npm run tauri:dev
```

HMR development mode — best for prototyping and UI refinement:

- Vite dev server starts on `http://localhost:5173` with full HMR support
- Rust compiles in debug mode (fast iteration, slower runtime)
- Edit any Svelte/TS/CSS file and changes appear instantly in the app window
- Rust changes require an app restart (re-run the command)

Uses the installed Metal Toolchain for GPU shader precompilation at build time.

### Release Binary

```bash
npm run tauri:build
```

Binary: `src-tauri/target/release/mrk-ragtag-app` (~110MB, arm64)

Self-contained — no runtime dependencies beyond macOS system libraries. The binary includes precompiled Metal shaders for instant first-launch GPU acceleration. Model downloads still occur on first run via the in-app progress overlay.

### App Bundle (Distributable)

```bash
npm run tauri:bundle
```

App bundle: `src-tauri/target/release/bundle/macos/Ragtag.app`

Full macOS application bundle with Info.plist, icons, and bundled frontend assets. This is what you'd distribute to end users. Includes the same release binary as `tauri:build`.

### Other Commands

```bash
npm run check          # TypeScript type-checking + Svelte diagnostics
npm run build          # Frontend-only Vite build (check output in frontend/dist/)
```
