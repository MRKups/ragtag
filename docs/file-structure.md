# File Structure

This document outlines the current layout of the application.

## Root Directory

- `package.json`: NPM workspace / scripts for driving both Svelte and Tauri builds.
- `opencode.jsonc`: Agent tooling configuration.
- `docs/`: Architecture, specification documentation, and implementation tracker.
- `frontend/`: The SvelteKit UI.
- `src-tauri/`: The Tauri Rust application shell.

## `frontend/` (SvelteKit UI)

The frontend is a SvelteKit application adapted to run inside a Tauri WebView as a Single Page Application (SPA).

- `svelte.config.js`: Configured for `adapter-static` with SPA fallback (`index.html`).
- `vite.config.ts`: Configured with `strictPort: true` and Tauri-specific dev server ignores.
- `src/routes/`: Client-side rendered pages. Currently contains the entry point (`+page.svelte`) with a simple Tauri IPC `ping` test.
- `src/lib/`: Svelte library files and assets.

## `src-tauri/` (Tauri App Shell)

The Tauri adapter layer connecting the OS, the UI, and the Rust backend.

- `tauri.conf.json`: Core configuration, permissions, and build instructions. Includes the `assetProtocol` configuration scoped to `$APPLOCALDATA`.
- `capabilities/`: JSON files defining exact capabilities mapped to specific window labels.
  - `default.json`: Grants the main window essential baseline permissions.
- `Cargo.toml`: Defines the Rust workspace, including the root Tauri app and the library crates in `crates/`.

### `src-tauri/src/`
- `main.rs`: Barebones entry point calling `mrk_ragtag_app::run()`.
- `lib.rs`: Core setup logic. Currently registers a basic `ping` Tauri command and initializes required plugins (`tauri_plugin_fs`, `tauri_plugin_shell`).

### `src-tauri/crates/` (Business Logic Workspace)

Pure Rust library crates that are completely unaware of Tauri, allowing portable and fast-testing business logic.

- `crates/domain/`: 
  - Scaffolding exists, but currently contains only default Rust library boilerplate. Will eventually hold shared types, state structs, and `thiserror` application errors.

---