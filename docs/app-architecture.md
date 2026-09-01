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
- Renders PDFs directly from disk using the browser's native renderer via Tauri's custom asset protocol (`http://asset.localhost/...` or `asset://localhost/...`). This is configured with `assetProtocol` in `tauri.conf.json`, not a broad filesystem capability; frontend code constructs asset URLs only through `convertFileSrc()`. Windows WebView2 disables the native PDF viewer by default (`msPdfOOUI`); ship `pdf.js` for consistent cross-platform rendering rather than relying on WebView quirks.

### Rust Core (The Backend)
- **Commands:** Thin `#[tauri::command]` functions acting as IPC handlers.
- **State & Setup (`lib.rs`):** The `lib.rs` file orchestrates state initialization, SQLite pooling, command registration, and background channels. It initializes `fastembed` in `spawn_blocking`; when local inference is selected, it asynchronously downloads and builds a pre-quantized GGUF `mistral.rs` model. FTS indexes are created after vector rows exist; IVF-PQ is deferred until the shared table contains its 256-vector training minimum. Model download/build failures are emitted as progress errors rather than falsely reporting the local model ready.
- **Job Queue:** A `tokio::sync::mpsc` channel runs in the background with a fixed capacity of `32` (`tokio::sync::mpsc::channel(32)`). Uploads use `try_send()` at the IPC boundary (returning `QueueFull` rather than blocking the UI); recovery uses `send().await` so recovered jobs are not silently dropped. A transient registry read after dequeue is re-enqueued by a separate bounded-backoff producer, never by blocking the sole worker.
- **Crash Recovery:** Startup completes durable `deleting` document records before normal work. After the embedder becomes ready, it requeues documents left `embedding`, retryable failed documents, and queued documents. It also requeues all indexed documents when the global `extraction_version` marker differs, so page-aware extraction changes are reapplied.
- **LLM Management:** `AppState` holds an optional `Arc<mistralrs::Model>` behind an async lock. Generation-tagged initialization serializes inference-mode transitions: invalidated initializers exit without publishing stale readiness or models. External mode releases any local model before emitting `llm_ready`; returning to local initializes the current persisted settings.
- **RAG Pipeline:** Rust generates query embeddings locally, searches LanceDB with hybrid search, requires at least one indexed non-empty retrieved excerpt, then builds the context prompt. An `InferenceProvider` enum selects in-process `mistral.rs` or external OpenAI-compatible transport and normalizes it into one Channel-forwarding path.
  - **Chunking Defaults:** Documents are split at the active EmbeddingGemma tokenizer’s boundaries: **512 tokens** per chunk with an **80-token** overlap, configurable through `chunk_size` and `chunk_overlap`. The `passage: ` prefix and special tokens count against the active 2,048-token embedding limit; no embedded input is silently truncated. The effective overlap is capped at half of a chunk to ensure forward progress.
  - **Embedding Prefixes:** Chunks must be prefixed with `"passage: "` during ingest, and user queries with `"query: "` to maximize retrieval quality with Instruction-aware models like EmbeddingGemma.

### SQLite & Embedded Vectors
- SQLite stores document metadata such as filename, status, chunk count, timestamps, and persisted processing errors.
- Embedded vector search (LanceDB) operates on local files, mapping 768-dimensional embeddings to document chunks without requiring a standalone database server.
- **Indexes and Fallbacks:** First creation of the shared `chunks` table is serialized; after an upsert, LanceDB creates FTS once rows exist. IVF-PQ is created only when the shared table reaches 256 vectors; smaller stores use LanceDB's flat vector search. If hybrid FTS/dense search fails, the backend falls back to vector-only search and sends a request-scoped warning to the query UI. RRF hybrid ordering is not rendered as calibrated citation confidence.

## 3. Data Flows

### Document Upload & Processing
1. User drops a PDF into the Svelte UI.
2. Svelte percent-encodes the UTF-8 filename in the ASCII `X-Filename` IPC header and sends the raw file bytes; Rust decodes and validates it before persistence.
3. Rust saves the file to the OS's specific App Data directory.
4. Rust inserts a `queued` row into SQLite and emits a job down the `tokio` channel.
5. Svelte receives the new document ID and listens for `ingestion_progress` stage events plus terminal document events.
6. A background `tokio` task consumes the job:
   - Extracts text using the `pdf-extract` crate, then verifies that its per-page result count matches the physical PDF page count before blank pages are filtered. A partial page-extraction result fails indexing rather than silently losing citation pages.
   - Chunks text.
   - Generates embeddings locally using `fastembed`.
   - **Vector lifecycle:** Deletes the document's prior chunks, then upserts replacement chunks into LanceDB.
   - **Write Order Discipline:** Marks the SQLite row `indexed` only after the vector upsert succeeds. A failed replacement can therefore leave a failed document without vectors until a retry, rather than an indexed document with missing vectors.

7. Rust emits a `document_indexed` Tauri Event back to Svelte to update the UI instantly.

### Document Deletion

1. Rust marks the SQLite registry row `deleting`, which excludes it from retrieval before any derived data is removed.
2. It deletes the document's LanceDB chunks, stored PDF, and finally the registry row.
3. A failed or interrupted deletion remains `deleting`; startup retries it before normal document work.

### Query Execution (RAG)
1. User types a query in the UI.
2. Svelte calls `invoke('execute_query', { text: "..." })`.
3. Rust generates the query embedding locally via `fastembed` (with `"query: "` prefix).
4. Rust searches the embedded Vector DB via **Hybrid Search** (native LanceDB FTS + dense vector search). If the FTS path fails, it falls back to vector-only search and streams a visible request-scoped warning before the answer.
5. Rust retrieves associated document metadata from SQLite, discards non-indexed or blank excerpts, and returns a typed no-evidence error without invoking inference when no usable citation remains.
6. Rust builds the chat messages, selects an `InferenceProvider` enum, and normalizes source-specific local `mistral.rs` or external SSE output into token, terminal, and streaming-error events.
7. One shared forwarder maps those events to ordered Tauri v2 `Channel<TokenEvent>` messages. Request and HTTP-status failures before external streaming reject the command; local stream-start failures follow the same typed IPC failure path, while failures after either stream starts send `TokenEvent::Error`.
8. Query state and its request-scoped Channel callback live in the application singleton, not the keyed query page. Hash-route navigation remounts the page without cancelling an active request or clearing completed text, citations, warnings, or selected pages; only the explicit query cancel control ends and resets that state.


**Citation Engine (Retrieval-Grounded)**
To eliminate hallucination risks, the application uses a retrieval-grounded citation approach (Option A). The LLM is *not* asked to generate page numbers. Instead, Rust attaches the top-K chunk metadata (which already contains the exact `page` integer) directly to the final `TokenEvent::Done` payload as structured citations. The UI then displays these precise sources (e.g., "Sources: page 4, page 11").

### LLM Lifecycle

`mistral.rs` runs in-process inside the main Rust binary. There is no subprocess, no port, no auth token, no health check. The lifecycle is:
- **Startup:** During `lib.rs` setup, local mode asynchronously downloads and builds the configured pre-quantized GGUF model (`GgufModelBuilder::new(repo_dir, vec![filename]).build().await`) from `$APPLOCALDATA/models`. Its tokenizer is resolved through Hugging Face's cache; when the user explicitly sets `HF_HUB_CACHE` or `HF_HOME`, that external cache remains outside Ragtag reset and storage accounting.
- **Ready Signal and Recovery:** A successful local build emits `llm_ready` and unlocks querying. Download or build failures emit `model_download_progress` with an error and leave querying unavailable; once the embedder is ready, the query UI exposes a local-model retry action. The retry invokes the typed `retry_local_model_initialization` command, reuses resumable downloads, and is disabled until it reaches either another progress error or `llm_ready`. Ingestion and Settings remain available throughout.
- **Runtime:** Tauri commands clone the optional model and call `stream_chat_request` only when it is present. The model drops with `AppState` at process exit.
- **Shutdown:** Nothing to do. The `Model` drops when `AppState` drops at process exit.

The previous machinery — ephemeral hex tokens, dynamic port binding, `--port N` arg parsing, `GET /health` polling, `Authorization: Bearer` headers, `command-group` process groups, `killpg`, `llama.lock` PID files, `RunEvent::CloseRequested` cleanup hooks — is all gone.

### BYO Inference Mode (External LLM)

Users can opt to use an external OpenAI-compatible endpoint instead of the in-process `mistral.rs` model:
- Managed by `inference_mode = "external"` and `external_model` in `app_settings`; an absent model falls back to `gpt-4o-mini`.
- `external_base_url` is stored in local SQLite, while the optional `external_api_key` is stored in the OS keychain under `service = "ragtag"` and `username = "external_api_key"`.
- External mode skips local model download/build and emits `llm_ready` immediately. That signal means the external route was selected, not that its endpoint, model, or credentials were verified.
- Retrieval, prompt construction, citation assembly, provider selection, and Channel event forwarding are shared. The provider enum keeps SSE parsing and `mistral.rs` response decoding source-specific; request and HTTP-status failures reject the IPC command, while failures after either response stream starts send `TokenEvent::Error`.

### Authentication
**Removed.** Since this is a local application running under the user's OS profile, cloud-style authentication (cookies, sessions, Postgres tables) is no longer required.

## 4. Packaging and Model Downloads

### Model Files and First-Run Staged Availability
Instead of bundling massive AI model files into the application binary, model files are downloaded on first run. To ensure a good UX, the application uses **Staged Availability**:
- **Embedding Models:** Downloaded via `fastembed`. Once initialized, the application emits `embedder_ready`, makes ingestion available, and lets the main application render.
- **Text Generation:** In local mode, the GGUF file is downloaded with `hf-hub` and then loaded by `mistral.rs`; querying remains disabled until `llm_ready`. In external mode, local generation initialization is skipped and `llm_ready` is emitted immediately. “Delete app-managed local model downloads” removes only the application-local GGUF/model entries, never a user-selected Hugging Face cache.
- **Boot progress and retry:** Local-model download emits byte counts from `hf-hub`; the overlay is determinate while totals are known and indeterminate while metadata is fetched or the model is built. A local-model error leaves an actionable query-unavailable message and explicit retry rather than blocking ingestion or requiring an application reload.
- **Boundary:** Query availability does not validate an external endpoint before the first request. Changing inference mode transitions query readiness and starts local initialization when needed.

### Retrieval Lifecycle Limits

- The page-aware migration uses one global `extraction_version` setting. Neither SQLite document rows nor LanceDB chunks record the extraction/chunk configuration that produced them.
- LanceDB has no query-time join to SQLite document status. Retrieval overfetches a bounded candidate set, then `execute_query` discards candidates whose documents are not `indexed`; exceptionally stale-heavy stores can still reduce the usable result set.
- Active queries register request-scoped cancellation senders. `cancel_query` interrupts pending external HTTP/local stream reads by dropping their futures; already-running blocking embedding work remains non-interruptible.
