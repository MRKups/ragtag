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
| **Storage** | Local OS Filesystem | Original user-owned source documents in the AppData directory |
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
- **Crash Recovery:** Startup completes durable `deleting` document records before normal work. After the embedder becomes ready, it requeues documents left `embedding`, retryable failed documents, and queued documents. Under the document-operations write lock, it atomically changes only `indexed` rows whose successful-index provenance is absent or differs from the desired extraction version, chunk size, or overlap; it enqueues those rows after the transaction and lock release.
- **LLM Management:** `AppState` holds an optional `Arc<mistralrs::Model>` behind an async lock. Generation-tagged initialization serializes inference-mode transitions: invalidated initializers exit without publishing stale readiness or models. External mode releases any local model and emits `llm_ready` only after its endpoint’s bounded model-list probe succeeds; returning to local initializes the current persisted settings.
- **RAG Pipeline:** Rust generates query embeddings locally, searches LanceDB with hybrid search, requires at least one indexed non-empty retrieved excerpt, then builds the context prompt. An `InferenceProvider` enum selects in-process `mistral.rs` or external OpenAI-compatible transport and normalizes it into one Channel-forwarding path.
  - **Chunking Defaults:** Documents are split at the active EmbeddingGemma tokenizer’s boundaries: **512 tokens** per chunk with an **80-token** overlap, configurable through `chunk_size` and `chunk_overlap`. The `passage: ` prefix and special tokens count against the active 2,048-token embedding limit; no embedded input is silently truncated. The effective overlap is capped at half of a chunk to ensure forward progress.
  - **Embedding Prefixes:** Chunks must be prefixed with `"passage: "` during ingest, and user queries with `"query: "` to maximize retrieval quality with Instruction-aware models like EmbeddingGemma.

### SQLite & Embedded Vectors
- SQLite stores document metadata, status, chunk count, timestamps, persisted processing errors, and the extraction/chunk provenance recorded only after successful vector replacement.
- Embedded vector search (LanceDB) operates on local files, mapping 768-dimensional embeddings to document chunks without requiring a standalone database server.
- **Indexes and Fallbacks:** First creation of the shared `chunks` table is serialized; after an upsert, LanceDB creates FTS once rows exist. IVF-PQ is created only when the shared table reaches 256 vectors; smaller stores use LanceDB's flat vector search. If hybrid FTS/dense search fails, the backend falls back to vector-only search and sends a request-scoped warning to the query UI. RRF hybrid ordering is not rendered as calibrated citation confidence.

## 3. Data Flows

### Document Upload & Processing
1. User drops a PDF, DOCX, or UTF-8 text file into the Svelte UI.
2. Svelte creates a request-scoped Tauri v2 `Channel<IngestionProgress>` and submits a typed upload request with the UTF-8 filename and raw file bytes.
3. Rust validates the source format, saves the original file to the OS's specific App Data directory, and inserts a `queued` row into SQLite.
4. Rust enqueues an in-memory job containing the document ID and, for this upload, its progress channel.
5. A background `tokio` task consumes the job and bridges its non-blocking pipeline updates to the request channel. The Document UI stores those ordered updates by document ID, so simultaneous uploads cannot overwrite or reorder each other's progress.
6. The worker:
   - Emits extraction progress after each physical PDF page is extracted, including blank pages; DOCX and non-paginated UTF-8 text sources use one logical source page.
   - Emits chunking progress after each nonblank source page has been tokenized, with the actual per-page and cumulative chunk totals.
   - Emits embedding progress after each completed 32-chunk model batch, with actual completed and total batches/chunks.
   - Emits `stored` only after vector replacement and the SQLite indexed/provenance update both succeed.
   - Continues to emit the `ingestion_progress` stage events (`extracting`, `chunking`, `embedding`, `storing`) for lifecycle compatibility; they are not the ordered UI progress path.
7. Rust emits a `document_indexed` Tauri Event back to Svelte to refresh durable document metadata instantly.
8. Citation viewing renders PDFs through bundled PDF.js. `.txt` previews use `get_text_preview(document_id)`: Rust validates the indexed registry record and safe app-local path, reads at most 1 MiB on a blocking worker, and returns typed escaped text for Svelte to render. Other non-PDF sources remain retrieval evidence and show an explicit preview-unavailable state until they have a format-specific renderer.


### Document Deletion

1. Rust marks the SQLite registry row `deleting`, which excludes it from retrieval before any derived data is removed.
2. It deletes the document's LanceDB chunks, stored original source, and finally the registry row.
3. A failed or interrupted deletion remains `deleting`; startup retries it before normal document work.

### Query Execution (RAG)
1. User types a query in the UI.
2. Svelte calls `invoke('execute_query', { text: "..." })`.
3. Rust generates the query embedding locally via `fastembed` (with `"query: "` prefix).
4. Rust searches the embedded Vector DB via **Hybrid Search** (native LanceDB FTS + dense vector search). If the FTS path fails, it falls back to vector-only search and streams a visible request-scoped warning before the answer.
5. Rust retrieves associated document metadata from SQLite, discards non-indexed or blank excerpts, and returns a typed no-evidence error without invoking inference when no usable citation remains.
6. Rust builds the chat messages, selects an `InferenceProvider` enum, and normalizes source-specific local `mistral.rs` or external SSE output into token, terminal, and streaming-error events.
7. One shared forwarder maps those events to ordered Tauri v2 `Channel<TokenEvent>` messages. Request and HTTP-status failures before external streaming reject the command; a local request-start failure and failures after either stream starts send `TokenEvent::Error` with the retrieved citations.
8. Query state and its request-scoped Channel callback live in the application singleton, not the keyed query page. A terminal `done` event preserves the completed text and citations while clearing the composer; failed requests retain the composer text, while explicit cancellation resets it. Hash-route navigation does not cancel an active request or clear completed text, citations, warnings, or selected pages.


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
- `external_base_url` is stored in local SQLite, while the optional `external_api_key` is stored in the OS keychain under `service = "ragtag"` and `username = "external_api_key"`. The key is never persisted in SQLite or frontend storage, or included in logs or lifecycle payloads.
- External mode skips local model download/build, normalizes the configured credential-free HTTP(S) base URL, and uses the shared HTTP client to call its OpenAI-compatible `/models` endpoint with bearer authentication only when a key is configured. The Remote provider dialog collects protocol, server address, and optional port, then composes the conventional `/v1` base path; arbitrary version-path discovery is not an OpenAI-compatible standard. It tests that unsaved endpoint with an unsaved in-form API key when supplied, otherwise the key held in the OS keychain, and receives a sorted, deduplicated list of nonempty model IDs in an app-styled dropdown. Testing does not persist a draft or emit lifecycle events. Changing endpoint controls clears the discovered models and selection. The user selects one returned model and saves both values through one SQLite transaction, then external mode runs one readiness validation. A successful response must list the saved model before `llm_ready` emits.
- A failed validation emits `external_inference_failed` with the serializable typed `AppError` payload (`kind`, `message`), leaves querying unavailable, and does not activate local download or retry UI. `execute_query` repeats the readiness probe before beginning an external inference request so stale lifecycle events cannot bypass validation.
- Retrieval, prompt construction, citation assembly, provider selection, and Channel event forwarding are shared. The provider enum keeps SSE parsing and `mistral.rs` response decoding source-specific; request and HTTP-status failures reject the IPC command, while failures after either response stream starts send `TokenEvent::Error`.

### Recoverable Failure Presentation

Frontend operation failures are reported through one route-independent, accessible error dialog owned by the application shell. Each notice has safe, task-specific copy and may provide a direct recovery action; raw transport strings, local URLs, and backend error payloads are not rendered in page content. The dialog queues distinct failures and deduplicates repeated lifecycle reports while they remain active. Query failures inspect the typed `AppError` message so a missing-evidence rejection directs users to indexed documents even when external inference is selected, rather than misreporting the provider as unavailable. In particular, an external readiness failure opens a “Remote provider unavailable” notice whose action navigates directly to the Remote provider configuration dialog. Query stream failures retain already-produced text and retrieval-grounded citations, while their failure notice explains that generation stopped. Document, upload, settings, keychain, main…

The full-screen initialization gate remains the exception: if the embedder, desktop IPC listener registration, or first-run initialization prevents the main UI from becoming usable, the gate provides safe startup guidance and a reload action instead of presenting an in-app dialog. A root `error`/`unhandledrejection` handler reports unexpected client failures through the dialog, excluding expected cancellation rejections.

### Authentication
**Removed.** Since this is a local application running under the user's OS profile, cloud-style authentication (cookies, sessions, Postgres tables) is no longer required.

## 4. Packaging and Model Downloads

### Model Files and First-Run Staged Availability
Instead of bundling massive AI model files into the application binary, model files are downloaded on first run. To ensure a good UX, the application uses **Staged Availability**:
- **Embedding Models:** Downloaded via `fastembed`. Once initialized, the application emits `embedder_ready`, makes ingestion available, and lets the main application render.
- **Text Generation:** In local mode, the GGUF file is downloaded with `hf-hub` and then loaded by `mistral.rs`; querying remains disabled until `llm_ready`. In external mode, local generation initialization is skipped and `llm_ready` follows only a successful bounded `/models` readiness probe for the configured/default model. “Delete app-managed local model downloads” removes only the application-local GGUF/model entries, never a user-selected Hugging Face cache.
- **Boot progress and retry:** Local-model download emits byte counts from `hf-hub`; the overlay is determinate while totals are known and indeterminate while metadata is fetched or the model is built. A local-model error leaves an actionable query-unavailable message and explicit retry rather than blocking ingestion or requiring an application reload. External readiness failures use their dedicated non-retry error state.
- **Boundary:** Query availability is gated by local model initialization or external endpoint/model validation. External query commands validate readiness again before their inference stream begins.

### Retrieval Lifecycle Limits

- Under the document-operations read lock, `execute_query` snapshots SQLite `indexed` document IDs and passes an escaped `document_id IN (...)` predicate to both hybrid and vector-only LanceDB queries before their candidate limit. It still verifies each returned registry row is `indexed` before creating citations; the lock makes that final check defensive rather than allowing a deletion or reindex transition to starve top-K retrieval.
- Active queries register request-scoped cancellation senders. `cancel_query` interrupts pending external HTTP/local stream reads by dropping their futures; already-running blocking embedding work remains non-interruptible.
