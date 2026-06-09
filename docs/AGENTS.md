# Agent House Rules & Non-Negotiables

These rules are strictly enforced for all AI agents and developers working on this project. They reflect the current "blessed" Rust and Tauri v2 ecosystem as of 2026.

## 1. The Supreme Tauri v2 Directive
**CRITICAL: Embed this in your mental model before writing any code:**
> We are on Tauri v2, `@tauri-apps/api` v2, `@sveltejs/vite-plugin-svelte`. Do NOT import from `@tauri-apps/api/fs`, `@tauri-apps/api/shell`, `@tauri-apps/api/window`, or `@tauri-apps/api/tauri` — those are v1 paths. Use `@tauri-apps/plugin-fs`, `@tauri-apps/plugin-shell`, `@tauri-apps/api/webviewWindow`, `@tauri-apps/api/core` respectively. Config uses `build.frontendDist`/`build.devUrl`, not `build.distDir`/`build.devPath`. Rust uses `get_webview_window`, not `get_window`.

## 2. Architecture & IPC
- **Commands are Handlers:** Treat `#[tauri::command]` as the security-scoped HTTP handler. Do NOT build an HTTP server (no `axum` or `actix`).
- **No Sidecars, Period:** All inference (embeddings via `fastembed`, generation via `mistral.rs`) runs in-process. We do NOT use `tauri-plugin-shell`'s sidecar capability, and there is no `src-tauri/binaries/` directory. If a future feature requires an external binary, that decision must be re-litigated against the simplicity tenet — the default answer is no.
- **SPA Frontend:** Plain Svelte 5 SPA compiled by Vite. Uses hash-based routing and `document.startViewTransition()` for same-document page transitions. Do NOT use SvelteKit patterns (`+page.svelte`, `+layout.svelte`, `load()` functions, SSR, prerendering, `@sveltejs/kit`, `adapter-static`) — this is a client-only SPA where only Tauri runtime APIs are available.

## 3. Rust Ecosystem & Crate Versions
Pin versions aggressively. Do not hallucinate older versions.

```toml
tauri         = "2"          # currently 2.8.x stable
lancedb       = "=0.26.2"    # pin exact — breaking changes per minor
fastembed     = "=5.12.0"    # pin exact — ort RC transitive dep
mistralrs     = "=0.8.1"     # pin exact — pre-1.0, breaking changes per minor
sqlx          = { version = "=0.8.6", features = ["sqlite", "runtime-tokio", "migrate"] }
arrow-array   = "57.2"       # workspace-level pin to avoid dual-major
arrow-schema  = "57.2"
tokio         = { version = "1", features = ["full"] }
reqwest       = { version = "0.12", default-features = false, features = ["rustls-tls", "stream", "json"] }
thiserror     = "2"
jiff          = "0.2"
keyring       = "3"          # explicitly enable apple-native + windows-native features
dotenvy       = "0.15"
rand          = "0.9"        # for non-crypto use only
uuid          = { version = "1", features = ["v7", "serde"] }
hf-hub        = "0.4"
```

- **Async:** `tokio` 1.x (use `features = ["full"]`). Prefer `async fn` in traits (native) over `#[async_trait]`.
- **Database:** `sqlx` 0.8 (Offline mode enabled, no ORMs). When adding `sqlx`, explicitly enable `features = ["sqlite", "runtime-tokio", "migrate"]` and **do not enable the chrono feature**. Map DATETIME columns to `String` in `sqlx` queries and parse with `jiff` at the domain layer. When inserting or querying `BLOB` columns (like `file_hash`), you MUST explicitly cast byte vectors using `.as_slice()` (e.g., `hash.as_slice()`) to prevent macro inference failures.
- **HTTP/API Calls:** `reqwest` 0.12 (Use `rustls-tls` instead of native-tls to avoid OpenSSL dependencies). Used exclusively for the BYO external inference endpoint — there is no longer any internal HTTP traffic since `mistral.rs` runs in-process.
- **Local Embeddings:** Use `fastembed` 5 (model: `EmbeddingModel::EmbeddingGemma300M`, 768-dim output via ONNX backend) for native, in-process embeddings. Note: `fastembed`'s `embed()` AND `try_new()` are **synchronous blocking calls** that hold the thread regardless of ONNX's internal thread pool; they MUST be run inside `tokio::task::spawn_blocking` to avoid starving the async executor. Cap embedding concurrency with `Semaphore::new(num_cpus::get().min(4))` during bulk ingest to avoid starving the DB. *Crucial: `fastembed` 5 pins `ort = "=2.0.0-rc.11"`. This is expected; do not wait for a stable 2.0.0. Also, Apple Silicon (M-series) acceleration is absent for the ONNX path—budget for CPU inference out of the box.*
- **Local LLM (Generation):** Use `mistralrs` 0.8.x for in-process text generation. **CRITICAL FOR APPLE SILICON:** You must conditionally enable the `metal` feature in `Cargo.toml` (`[target.'cfg(target_os = "macos")'.dependencies]`) to access the GPU/ANE unified memory, otherwise the model will fail to fit in the CPU context. Building with the `metal` feature requires the Xcode Metal Toolchain (`sudo xcodebuild -downloadComponent MetalToolchain`), or you can prefix builds with `MISTRALRS_METAL_PRECOMPILE=0` to skip build-time shader compilation. Build the model once at app startup via `GgufModelBuilder` for GGUF weights, wrap the resulting `Model` in `Arc<Model>`, and store it in `AppState`. Use `model.stream_chat_request(messages).await?` for streaming responses; pump `Response::Chunk` deltas into the `Channel<TokenEvent>` returned to the frontend. **Construction is async but heavy** — it loads weights and allocates KV cache; do this exactly once during the staged-availability boot sequence and emit `llm_ready` only after `.build().await` resolves. GGUF models are pre-quantized — ISQ does not apply in local mode.
- **Model Downloads:** Use `hf-hub` 0.4 for downloading model files on first run with resume support (do not bundle models). Set explicit cache dirs for models using `$APPLOCALDATA/models`. `mistral.rs`'s builders will resolve from this cache automatically when given a HF repo ID, but pre-download with `hf-hub` so we control the progress UI.
- **Security & Tokens:** Use `keyring` 3.x for storing external API keys in the OS keychain. `Entry::set_password` silently upserts on all platforms; *never* use a delete-then-set pattern. **No internal auth tokens are needed** — there is no sidecar, no localhost HTTP server, no port to protect. The previous `getrandom` + ephemeral hex token machinery is gone.
- **Vector DB:** `lancedb` 0.26 — pin this aggressively. Use a `tokio::sync::Semaphore(1)` for all writes to prevent `Commit conflict for version N`. Also use `merge_insert` for idempotent upserts.
- **SQLite Setup:** Always use `app.path().app_local_data_dir()` for SQLite and call `std::fs::create_dir_all` on the parent directory before opening the pool to prevent cryptic "unable to open database" errors.
- **Time/Dates:** `jiff` 0.2+ (Do not use `chrono`).
- **Error Handling:** `thiserror` 2.x for library/domain errors, mapped to custom serialize types for Tauri commands.
- **sqlx 0.8 Transaction Bug:** Never race `pool.begin()` against a `tokio::select!` or timeout branch. If a transaction setup future is dropped, it permanently poisons the connection. Drop and rebuild the pool if you hit a non-zero transaction depth error.
- **macOS Code Signing:** With the sidecar gone, the only signing concern is `onnxruntime.dylib` (transitively bundled by `fastembed`/`ort`). It must be signed with the hardened runtime and the `com.apple.security.cs.disable-library-validation` entitlement before notarization. `mistral.rs` is pure Rust (Candle backend) and links statically — no extra dylib to sign.
- **Env Config:** `dotenvy` 0.15 (Do NOT use the abandoned `dotenv` crate).

## 4. Concurrency & Safety
- **Blocking vs Async:** Never perform blocking operations (`std::fs`, `std::thread::sleep`, sync DB calls) in an async Tauri command. Use `tokio::task::spawn_blocking` for CPU-bound or blocking I/O work, or use `tokio::fs`. `mistral.rs`'s request methods are already async-friendly — do **not** wrap them in `spawn_blocking`; they internally dispatch to the engine's worker threads via mpsc.
- **Mutex Discipline:** Default to `std::sync::Mutex` for state. ONLY use `tokio::sync::Mutex` if the lock is held across an `.await` point. Beware of `parking_lot::Mutex` causing silent deadlocks across `.await`. The `mistralrs::Model` is internally `Send + Sync` and shareable via `Arc` — do not wrap it in a `Mutex`.
- **Background Jobs:** Use `tokio::sync::mpsc` channels to decouple UI commands from heavy PDF processing tasks. *Critical:* Inside `#[tauri::command]`, you MUST use `tx.try_send()` (returning a typed error on `Full`) to enqueue jobs. Using `send().await` inside a command will deadlock the UI if the queue is full. Inside background tasks, use `send().await` for backpressure. Use `tauri::ipc::Channel<T>` for progress events, not `app.emit`.
- **LLM Concurrency:** `mistral.rs` handles its own continuous batching internally. Do **not** put a `Semaphore` in front of `stream_chat_request` — let the engine schedule. The only thing the app gates is the ingestion pipeline (embeddings + DB writes).

## 5. Error Serialization for Frontend
Tauri commands must return `serde::Serialize` errors. Do not return opaque `Result<T, String>`. Define a domain error enum and implement `Serialize` manually to pass structured `{ kind, message }` objects back to Svelte.

```rust
// In crates/domain/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Inference error: {0}")]
    Inference(String),
    // ...
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self { AppError::Database(e.to_string()) }
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("kind", &format!("{:?}", std::mem::discriminant(self)))?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}
```
All `#[tauri::command]` functions return `Result<T, AppError>`. The `Serialize` derive with `serde(tag = "kind")` gives the frontend a stable discriminated union to switch on.

## 6. Project Layout

- `frontend/`: Plain Svelte 5 SPA compiled by Vite. Hash-based routing, no SvelteKit. Pages live in `src/pages/`, shared components and stores in `src/lib/`. See `docs/file-structure.md` for the full breakdown.
- `src-tauri/`: The Tauri Rust application shell. Keep this thin — it should only contain `tauri.conf.json`, capabilities, and thin command wrapper functions in `src/`. Heavy business logic must live in separated library crates under `src-tauri/crates/domain`, `crates/rag_pipeline`, and `crates/persistence` to ensure fast testing and Tauri-agnostic logic.
- There is no `src-tauri/binaries/` directory — all inference is in-process.
