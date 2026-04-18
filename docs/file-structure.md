# File Structure (V4)

This layout follows the recommended mature pattern for Tauri v2 + Rust development, ensuring clean separation between the frontend, the Tauri wrapper, and the pure Rust business logic.

## Root Directory

- `package.json`: NPM workspace / scripts for driving both Svelte and Tauri builds.
- `docs/`: Architecture and specification documentation.

## `frontend/` (SvelteKit UI)

The exact same SvelteKit application, adapted to run inside a WebView.

- `svelte.config.js`: Configured for `adapter-static` with SPA fallback.
- `vite.config.ts`: Configured with `strictPort: true` for Tauri compatibility.
- `src/lib/ipc.ts`: Hand-typed wrappers or `tauri-specta` generated types for calling Rust commands.
- `src/routes/`: Client-side rendered pages. (API routes are completely removed).

## `src-tauri/` (Tauri App Shell)

The thin adapter layer that connects the OS, the UI, and the Rust business logic.

- `tauri.conf.json`: Core configuration, permissions, and build instructions. Includes the `assetProtocol` configuration scoped to `$APPLOCALDATA/**` and `$APPLOCALDATA/.**/**`.
- `capabilities/`: JSON files defining exact capabilities mapped to specific window labels.
  - `default.json`: Grants the main window essential permissions:
    - `fs:allow-read-file` / `fs:allow-write-file` scoped to the AppLocalData directory.
    - `shell:allow-execute` scoped to the `llama-server` sidecar binary name.
    - `core:allow-invoke` for all registered IPC commands.
- `Cargo.toml`: Defines the workspace and includes the `crates/` below.

### `src-tauri/binaries/`
- `llama-server-<target-triple>`: The compiled `llama.cpp` server executable, managed by Tauri's sidecar capability (e.g., `llama-server-aarch64-apple-darwin`).

### `src-tauri/src/`
- `main.rs`: Barebones entry point calling `lib::run()`.
- `lib.rs`: Setup logic, registering Tauri commands, initializing state, and spawning background `tokio` tasks.
- `commands/`: Thin IPC handlers (`#[tauri::command]`). These functions deserialize arguments, call the underlying `crates/`, map errors to serializable types, and return.

### `src-tauri/crates/` (Business Logic)

Pure Rust library crates that are completely unaware of Tauri. This makes tests fast and logic portable.

- `crates/persistence/`: 
  - Manages SQLite via `sqlx` and the embedded vector database.
  - Handles migrations and `documents` table CRUD.
- `crates/rag_pipeline/`:
  - Generates embeddings locally via `fastembed`.
  - Communicates with the `llama.cpp` sidecar for inference.
  - Handles PDF extraction, text chunking, and overlapping.
  - Implements the prompt building and answer synthesis.
- `crates/domain/`:
  - Shared types, structs, and custom `thiserror` enums representing application state and errors.
