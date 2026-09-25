# Contributing to Ragtag

Thank you for your interest in contributing to Ragtag! Ragtag is a local-first desktop application designed to provide intelligent, retrieval-grounded query answers over user-provided documents and technical manuals.

## Core Principles

When contributing to Ragtag, please keep our core development principles in mind:
1. **Simple Code**: Prefer clean, direct, understandable solutions over clever or prematurely optimized abstractions.
2. **Reliable Code**: Ingestion, persistence, and inference must be crash-safe, idempotent, and handle edge cases gracefully.
3. **Modular Code**: Respect the crate and application boundaries:
   - `frontend/`: Plain Svelte 5 runes, strict `api.ts` IPC isolation, no global store frameworks or CSS kits.
   - `crates/domain/`: Pure models, events, and typed errors. Free of Tauri, HTTP, or database dependencies.
   - `crates/persistence/`: SQLite registry and embedded LanceDB storage.
   - `crates/rag_pipeline/`: Extraction, chunking, and embeddings.
   - `src-tauri/src/`: Application shell, IPC handlers, lifecycle, and capacity management.
4. **Local-First & Grounded**: The application must run without external servers or accounts. All answers must ground in retrieval citations linked to physical document pages.

## Development Setup

### Prerequisites
- **Node.js**: `24.x` (LTS recommended)
- **Rust**: `1.98.0` or newer
- **Protobuf compiler**: `protoc` (`brew install protobuf` on macOS)
- **Task**: `go-task` (`brew install go-task/tap/go-task`)
- **Tauri CLI**: `cargo install tauri-cli --version "^2.0"`

On macOS Apple Silicon, ensure the Metal toolchain is available:
```bash
xcodebuild -downloadComponent MetalToolchain
```

### Getting Started
```bash
# 1. Clone the repository
git clone https://github.com/MRKups/mrk-ragtag.git
cd mrk-ragtag

# 2. Install dependencies
task setup

# 3. Start development mode (Vite HMR + Tauri desktop window)
task dev
```

## Running Tests

Before submitting changes, ensure all linters, type-checks, and test suites pass:

```bash
# Run the complete test suite (frontend tests + Rust workspace tests)
npm test

# Run frontend type-checking
npm run check --prefix frontend

# Run Rust linter across the workspace
TOOLCHAINS=Metal cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings
```

## Pull Request Guidelines

- **Scoped Changes**: Keep PRs focused on a single feature, bug fix, or reviewable intent.
- **Git Hygiene**: Stage explicit files only (avoid `git add -A`). Never commit build outputs, `.DS_Store` files, or local models.
- **Documentation**: Update documentation in `docs/` when modifying public contracts, storage schemas, or architectural flows.

For an in-depth reference on architectural decisions, consult [`docs/app-architecture.md`](docs/app-architecture.md) and [`docs/decisions.md`](docs/decisions.md).
