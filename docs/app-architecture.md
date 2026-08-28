# Application Architecture

## 1. System Overview

The application is a monolithic, secure, and blazing-fast desktop application using Tauri. This approach prioritizes privacy, local data control, and zero hosting costs.

| Component | Technology | Role |
|-----------|------------|------|
| **App Shell** | Tauri v2 | OS integration, window management, security capabilities |
| **Frontend** | Svelte 5 (Vite) | UI, running inside the OS-native WebView |
| **Core Engine** | Rust | Document processing, embedding logic, file I/O, API clients |
| **Relational DB** | SQLite (`sqlx`) | Document registry and app settings |
| **Vector DB** | LanceDB | Embedded semantic similarity search |
| **Storage** | Local OS Filesystem | Raw PDF storage in AppData/Config directories |
| **Local Embeddings** | `fastembed-rs` | In-process ONNX runtime for generating text embeddings locally |
| **Local LLM** | `mistral.rs` | In-process, pure-Rust inference engine for text generation |

## 2. Component Responsibilities

### Tauri & Svelte 5 (The Frontend)
- No longer makes HTTP requests to a backend server.
- Uses `invoke('command_name', { args })` to securely request actions from Rust via IPC.
- Renders PDFs directly from disk using the browser's native renderer via Tauri's custom asset protocol (`http://asset.localhost/...` or `asset://localhost/...`). **Note:** This is configured in `tauri.conf.json` using `assetProtocol`, not in capabilities. The scope must include `$APPLOCALDATA/**` and `$APPLOCALDATA/.**/**` (to match dotfiles like `.cache`). Always construct asset URLs via `convertFileSrc()`. Windows WebView2 disables the native PDF viewer by default (`msPdfOOUI`); it is highly recommended to ship `pdf.js` for consistent cross-platform rendering rather than relying on WebView quirks.

### Rust Core (The Backend)
- **Commands:** Thin `#[tauri::command]` functions acting as IPC handlers.
- **State & Setup (`lib.rs`):** The `lib.rs` file acts as the primary orchestrator. It handles application state initialization, SQLite connection pooling, Tauri command registration, and spawning the background `tokio` channels. It also initializes the `fastembed` model inside a `spawn_blocking` task to prevent UI freezing, asynchronously builds the `mistral.rs` `Model` (loading weights and running ISQ quantization), and runs an FTS index health check on LanceDB (rebuilding if missing). With no sidecar to babysit, `lib.rs` is meaningfully simpler — no port binding, no PID lockfiles, no health-check polling, no auth-token generation, no process-group setup.
- **Job Queue:** A `tokio::sync::mpsc` channel runs in the background with a fixed capacity of `32` (`tokio::sync::mpsc::channel(32)`). When documents are uploaded, messages are pushed here using `try_send()` at the Tauri IPC boundary (to prevent freezing the UI if the queue is full), returning a `QueueFull` error if rejected. Background tasks use `send().await` for backpressure.
- **Crash Recovery:** On startup, `lib.rs` queries SQLite for any documents with `status IN ('queued', 'embedding')`. It resets any `'embedding'` rows back to `'queued'` and pushes all recovered IDs back into the `mpsc` channel before the background worker begins, ensuring no jobs are silently lost after an app crash.
- **LLM Management:** Holds an `Arc<mistralrs::Model>` in `AppState`, built once during the staged-availability boot sequence. There is no process to manage, no socket to monitor, no cleanup hook required on shutdown — when the app process exits, the model drops with it.
- **RAG Pipeline:** Rust generates query embeddings locally via `fastembed`, searches the LanceDB vector database using Hybrid Search, builds the context prompt, and streams the response from the in-process `mistral.rs` model.
  - **Chunking Defaults:** Documents use character windows estimated at four characters per token: **512 estimated tokens** (about 2,048 characters) per chunk with an **80 estimated-token** (about 320-character) overlap. These settings do not create exact tokenizer boundaries. The effective overlap is capped at half of the window to ensure forward progress. Configure them with the `app_settings` keys `chunk_size` and `chunk_overlap`.
  - **Embedding Prefixes:** Chunks must be prefixed with `"passage: "` during ingest, and user queries with `"query: "` to maximize retrieval quality with Instruction-aware models like EmbeddingGemma.

### SQLite & Embedded Vectors
- SQLite stores all document metadata (`filename`, `status`, `page_count`, etc.).
- Embedded vector search (LanceDB) operates on local files, mapping 768-dimensional embeddings to document chunks without requiring a standalone database server.
- **FTS Index Creation:** Must be created AFTER rows are inserted (empty table creation leads to a silently broken index). Always use `.replace(true)` and `wait_for_index()` to ensure idempotency and observation. Hybrid search without an index *hard fails* (there is no silent fallback to vector-only).

## 3. Data Flows

### Document Upload & Processing
1. User drops a PDF into the Svelte UI.
2. Svelte passes the raw file bytes or filepath to Rust via IPC.
3. Rust saves the file to the OS's specific App Data directory.
4. Rust inserts a `queued` row into SQLite and emits a job down the `tokio` channel.
5. Svelte receives the new document ID and begins polling/listening for progress events.
6. A background `tokio` task consumes the job:
   - Extracts text using the `pdf-extract` crate. **Crucial:** Extraction must happen in a per-page loop to preserve physical page boundaries and ensure accurate page numbers are attached to chunks for the citation engine.
   - Chunks text.
   - Generates embeddings locally using `fastembed`.
   - **Write Order Discipline:** Saves embeddings to the vector DB (LanceDB) *first*.
   - Updates the SQLite row to `indexed` *second*. (A crash between these steps leaves idempotent vectors in LanceDB to be safely overwritten on restart).
7. Rust emits a `document_indexed` Tauri Event back to Svelte to update the UI instantly.

### Query Execution (RAG)
1. User types a query in the UI.
2. Svelte calls `invoke('execute_query', { text: "..." })`.
3. Rust generates the query embedding locally via `fastembed` (with `"query: "` prefix).
4. Rust searches the embedded Vector DB via **Hybrid Search** (native LanceDB FTS + dense vector search combined using Reciprocal Rank Fusion) for the top-K matching chunks. If the FTS index is missing or throws an error, it must catch the error and gracefully fall back to a vector-only search (emitting a warning event to the UI).
5. Rust retrieves associated document metadata from SQLite.
6. Rust builds the chat messages and calls `model.stream_chat_request(messages).await?` on the in-process `mistral.rs` model (or, if `inference_mode = "external"`, POSTs to the user's OpenAI-compatible endpoint via `reqwest`) to synthesize an answer based on the chunks.
7. Rust parses the citations and streams the response to Svelte using Tauri v2 Channels (`Channel<TokenEvent>`). For local inference, the loop pulls `Response::Chunk` deltas off the `mistral.rs` stream and forwards each `Delta::content` as a `TokenEvent::Token`. This ensures ordered, fast streaming with backpressure, avoiding older `app.emit` event mechanisms.

**Citation Engine (Retrieval-Grounded)**
To eliminate hallucination risks, the application uses a retrieval-grounded citation approach (Option A). The LLM is *not* asked to generate page numbers. Instead, Rust attaches the top-K chunk metadata (which already contains the exact `page` integer) directly to the final `TokenEvent::Done` payload as structured citations. The UI then displays these precise sources (e.g., "Sources: page 4, page 11").

### LLM Lifecycle

`mistral.rs` runs in-process inside the main Rust binary. There is no subprocess, no port, no auth token, no health check. The lifecycle is:
- **Startup:** During `lib.rs` setup, build the model asynchronously (`GgufModelBuilder::new(repo_dir, vec![filename]).build().await`). This is the only slow step — it loads pre-quantized weights from `$APPLOCALDATA/models`. Wrap the result in `Arc<Model>` and stash it in `AppState`.
- **Ready Signal:** Once `.build().await` resolves, emit `llm_ready` to the UI to unlock the query interface.
- **Runtime:** Tauri commands clone the `Arc<Model>` and call `stream_chat_request`. `mistral.rs` handles continuous batching, KV cache, and worker threads internally.
- **Shutdown:** Nothing to do. The `Model` drops when `AppState` drops at process exit.

The previous machinery — ephemeral hex tokens, dynamic port binding, `--port N` arg parsing, `GET /health` polling, `Authorization: Bearer` headers, `command-group` process groups, `killpg`, `llama.lock` PID files, `RunEvent::CloseRequested` cleanup hooks — is all gone.

### BYO Inference Mode (External LLM)

Users can opt to use an external OpenAI-compatible endpoint instead of the in-process `mistral.rs` model:
- Managed via `inference_mode = "external"` in `app_settings`.
- The `external_base_url` is stored in the local SQLite database.
- The `external_api_key` is securely stored in the OS keychain using the `keyring` crate, not in SQLite. Specifically, the keyring namespace must be set to `service = "ragtag"` and `username = "external_api_key"` to prevent collisions if the user runs multiple profiles.
- When configured, the RAG pipeline bypasses the local `mistral.rs` model entirely and routes inference requests to the user's endpoint via `reqwest`. The two paths share the same prompt-building logic and are unified behind a small `InferenceClient` trait so the streaming/citation glue is identical.
- Selecting external mode also skips the LLM weight download on first run (see Staged Availability below).

### Authentication
**Removed.** Since this is a local application running under the user's OS profile, cloud-style authentication (cookies, sessions, Postgres tables) is no longer required.

## 4. Packaging and Model Downloads

### Model Files and First-Run Staged Availability
Instead of bundling massive AI model files into the application binary, model files are downloaded on first run. To ensure a good UX, the application uses **Staged Availability**:
- **Embedding Models:** Downloaded via `fastembed` (or `hf-hub` with resume support). This is relatively small (~500MB). Once complete, the application emits an `embedder_ready` event, **unlocking the document ingestion UI** immediately.
- **Text Generation:** Model weights for `mistral.rs` (e.g., Qwen3-1.7B GGUF files) are downloaded to the OS application data directory using the `hf-hub` crate in parallel with the embedding model. Because LLMs can be large, the **query interface remains locked** until `mistral.rs`'s `GgufModelBuilder::build().await` completes and an `llm_ready` event fires.
- **Boot progress:** The local-model download emits byte counts from `hf-hub`; the boot overlay is determinate only while those totals are known and uses an indeterminate state while fetching metadata or constructing the model.
- **Rationale:** This prevents the user from waiting 10+ minutes on a slow connection before they can even upload a PDF. If BYO inference mode is active, the LLM download and in-process model build are skipped entirely and the query UI unlocks as soon as the embedder is ready.
