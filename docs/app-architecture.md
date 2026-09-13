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
- Renders citation PDFs from app-local storage through Tauri's custom asset protocol (`http://asset.localhost/...` or `asset://localhost/...`), configured with `assetProtocol` in `tauri.conf.json`; frontend code constructs asset URLs only through `convertFileSrc()`. Bundled legacy PDF.js uses a matching module worker because macOS JavaScriptCore lacks an API required by PDF.js's generic build. PDF.js rasterizes visible pages into device-pixel-ratio-aware canvases and overlays its text layer for selection. Document replacement unmounts outgoing pages before serializing PDF.js task teardown, so worker destruction cannot race a subsequent load.

### Rust Core (The Backend)
- **Commands:** Thin `#[tauri::command]` functions acting as IPC handlers.
- **State & Setup (`lib.rs`):** The `lib.rs` file orchestrates state initialization, SQLite pooling, command registration, and background channels. It initializes `fastembed` in `spawn_blocking`; when local inference is selected, it asynchronously downloads and builds a pre-quantized GGUF `mistral.rs` model. FTS indexes are created after vector rows exist; IVF-PQ is deferred until the shared table contains its 256-vector training minimum. Model download/build failures are emitted as progress errors rather than falsely reporting the local model ready.
- **Job Queue:** A `tokio::sync::mpsc` channel runs in the background with a fixed capacity of `32` (`tokio::sync::mpsc::channel(32)`). Uploads use `try_send()` at the IPC boundary (returning `QueueFull` rather than blocking the UI); recovery uses `send().await` so recovered jobs are not silently dropped. A transient registry read after dequeue is re-enqueued by a separate bounded-backoff producer, never by blocking the sole worker.
- **Crash Recovery:** Startup completes durable `deleting` document records and removes only unregistered direct files with canonical UUID v7 managed-source names, recovering the source-write-before-registry crash window without touching user files or directories. It verifies each indexed registry chunk count against the existing Lance table and requeues missing or partial vector data; search and compaction never create a missing table. A successful index or intentional indexed-document requeue resets its attempt counter. After the embedder becomes ready, it requeues queued documents, retryable failed documents, and interrupted `embedding` documents below the three-attempt limit for the current unsuccessful processing run; documents at that limit become terminal `failed` records rather than restarting the same work indefinitely. Under the document-operations write lock, it atomically changes only `indexed` rows whose successful-index provenance is absent or differs from the desired extraction version, chunk size, or overlap; it enqueues those rows after the transaction and lock release.
- **LLM Management:** `AppState` holds an optional `Arc<mistralrs::Model>` behind an async lock. Generation-tagged initialization serializes inference-mode transitions: invalidated initializers exit without publishing stale readiness or models. External mode releases any local model and emits `llm_ready` only after its endpoint’s bounded model-list probe succeeds; returning to local initializes the current persisted settings.
- **RAG Pipeline:** Rust bounds every extracted document's retained text to 16 MiB, plain-text source reads to that same cap, one document to 8,192 chunks, and prepared embedding input to a bounded amount; it verifies returned embeddings have the exact 768-dimensional shape before Arrow allocation. DOCX `word/document.xml` extraction remains bounded to 16 MiB using both ZIP metadata and streamed decompressed bytes. UTF-8 Markdown previews cap recursive block and inline structure at 32 levels before returning typed safe blocks; direct text in tight list items remains a paragraph. Rust generates query embeddings locally, searches LanceDB with hybrid search, requires at least one indexed non-empty retrieved excerpt, then builds the context prompt. An `InferenceProvider` enum selects in-process `mistral.rs` or external OpenAI-compatible transport and normalizes it into one Channel-forwarding path.
  - **Chunking Defaults:** Documents are split at the active EmbeddingGemma tokenizer’s boundaries: **512 tokens** per chunk with an **80-token** overlap, configurable through `chunk_size` and `chunk_overlap`. The `passage: ` prefix and special tokens count against the active 2,048-token embedding limit; no embedded input is silently truncated. The effective overlap is capped at half of a chunk to ensure forward progress.
  - **Embedding Prefixes:** Chunks must be prefixed with `"passage: "` during ingest, and user queries with `"query: "` to maximize retrieval quality with Instruction-aware models like EmbeddingGemma.

### SQLite & Embedded Vectors
- SQLite stores document metadata, status, chunk count, timestamps, persisted processing errors, and the extraction/chunk provenance recorded only after successful vector replacement.
- Embedded vector search (LanceDB) operates on local files, mapping 768-dimensional embeddings to document chunks without requiring a standalone database server.
- **Indexes and Fallbacks:** First creation of the shared `chunks` table is serialized; after an upsert, LanceDB creates FTS once rows exist. IVF-PQ is created only when the shared table reaches 256 vectors; smaller stores use LanceDB's flat vector search. If hybrid FTS/dense search fails, the backend falls back to vector-only search and sends a request-scoped warning to the query UI. RRF hybrid ordering is not rendered as calibrated citation confidence.

## 3. Data Flows

### Document Upload & Processing
1. User selects or drops one or more PDF, DOCX, or UTF-8 text files into the Svelte UI. Browser drag-and-drop is enabled in the Tauri WebView; root listeners prevent dropped files outside the upload zone from navigating the application.
2. Svelte stages every selected file, then creates a request-scoped Tauri v2 `Channel<IngestionProgress>` and submits one typed upload request at a time with the UTF-8 filename and raw file bytes.
3. Rust validates the source format, rejecting raw UTF-8 text above the 16 MiB indexing limit before storage. It writes accepted sources through a unique managed temporary file before atomically renaming them under a canonical UUID v7 name in the OS App Data directory, then inserts a `queued` SQLite row. The worker verifies the stored size and SHA-512 against that row before extraction; mismatch fails processing rather than indexing altered evidence. Insert failure removes the just-written file; startup reconciles a process-crash orphan only when that managed filename has no registry row.
4. Rust enqueues an in-memory job containing the document ID and, for this upload, its progress channel.
5. A background `tokio` task consumes the job and bridges its non-blocking pipeline updates to the request channel. The Document UI stores ordered updates by document ID; authoritative terminal lifecycle events finalize that ID before refreshing durable metadata, so a delayed channel callback cannot restore stale progress.
6. The worker:
   - Emits extraction progress after each physical PDF page is extracted, including blank pages; DOCX and non-paginated UTF-8 text sources use one logical source page.
   - Emits chunking progress after each nonblank source page has been tokenized, with the actual per-page and cumulative chunk totals.
   - Emits embedding progress after each completed 32-chunk model batch, with actual completed and total batches/chunks.
   - Emits `stored` only after vector replacement and the SQLite indexed/provenance update both succeed.
   - Continues to emit the `ingestion_progress` stage events (`extracting`, `chunking`, `embedding`, `storing`) for lifecycle compatibility; they are not the ordered UI progress path.
7. Rust emits a `document_indexed` Tauri Event back to Svelte to refresh durable document metadata instantly.
8. Citation viewing renders app-local PDF URLs through the matching legacy PDF.js main library and module worker. It rasterizes the cited page and nearby context into device-pixel-ratio-aware canvases, with a separate text-selection layer; switching documents unmounts outgoing pages before awaiting their PDF.js task teardown. Other previewable sources use `get_document_preview(document_id)`: Rust validates the indexed registry record and safe app-local path, reads at most 1 MiB on a blocking worker, and returns a typed model for Svelte to render. UTF-8 sources render verbatim except `.md`/`.markdown`, which Rust parses into safe headings, paragraphs, lists, quotes, tables, code, and allowlisted links after omitting leading YAML front matter from Markdown previews only; raw HTML, images, embeds, and unsafe links never cross IPC. DOCX previews return extracted text; if the XML read limit lands inside incomplete markup or an entity reference, parsing retains the already extracted text. Preview values are never generated HTML or arbitrary filesystem paths.


### Document Deletion

1. Rust marks the SQLite registry row `deleting`, which excludes it from retrieval before any derived data is removed.
2. It deletes the document's LanceDB chunks, stored original source, and finally the registry row.
3. A failed or interrupted deletion remains `deleting`; startup retries it before normal document work.

### Query Execution (RAG)
1. User types a query in the UI.
2. Svelte calls `invoke('execute_query', { query, requestId, channel })`, where `requestId` owns cancellation and the request-scoped `Channel<TokenEvent>` receives ordered output.
3. Rust generates the query embedding locally via `fastembed` (with `"query: "` prefix).
4. Rust searches the embedded Vector DB via **Hybrid Search** (native LanceDB FTS + dense vector search). If the FTS path fails, it falls back to vector-only search and streams a visible request-scoped warning before the answer.
5. Rust retrieves associated document metadata from SQLite, discards non-indexed or blank excerpts, and returns a typed no-evidence error without invoking inference when no usable citation remains.
6. Rust builds the chat messages, selects an `InferenceProvider` enum, and normalizes source-specific local `mistral.rs` or external SSE output into token, terminal, and streaming-error events.
7. One shared forwarder maps those events to the ordered Tauri v2 `Channel<TokenEvent>` supplied in the invocation. Request and HTTP-status failures before external streaming reject the command; a local request-start failure and failures after either stream starts send `TokenEvent::Error` with the retrieved citations.
8. Duplicate active request IDs are rejected. `cancel_query(requestId)` interrupts external readiness and stream reads or local stream reads; it cannot interrupt already-running blocking embedding work. Query state and its request-scoped Channel callback live in the application singleton, not the keyed query page. A terminal `done` event preserves the completed text and citations while clearing the composer; failed requests retain the composer text, while explicit cancellation resets it. Hash-route navigation does not cancel an active request or clear completed text, citations, warnings, or selected pages.


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
- External mode skips local model download/build, normalizes the configured credential-free HTTP(S) base URL, and uses a no-redirect shared HTTP client to call its OpenAI-compatible `/models` endpoint with bearer authentication only when a key is configured. Rust bounds query/request identifiers, `/models` metadata, complete SSE events, tokens, aggregate streamed output, and external request/idle time; local `mistral.rs` generation retains its own concurrency control. The Remote provider dialog collects protocol, server address, and optional port, then composes the conventional `/v1` base path; arbitrary version-path discovery is not an OpenAI-compatible standard. It tests that unsaved endpoint with an unsaved in-form API key when supplied, otherwise the key held in the OS keychain, and receives a sorted, deduplicated list of nonempty model IDs in an app-styled dropdown. Testing does not persist a draft or emit lifecycle events. Changing endpoint controls clears the discovered list and select…
- A failed validation emits `external_inference_failed` with the serializable typed `AppError` payload (`kind`, `message`), leaves querying unavailable, and does not activate local download or retry UI. `execute_query` repeats the readiness probe before beginning an external inference request so stale lifecycle events cannot bypass validation.
- Retrieval, prompt construction, citation assembly, provider selection, and Channel event forwarding are shared. The provider enum keeps strict bounded SSE parsing and `mistral.rs` response decoding source-specific; cancelling a request stops readiness probes and either provider stream. Request and HTTP-status failures reject the IPC command, while failures after either response stream starts send `TokenEvent::Error`.

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
