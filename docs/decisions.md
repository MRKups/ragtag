# Design Rationale

This document explains the core design and technology choices for the application.

## 1. Desktop Application (Tauri)

**Decision:** The application is a compiled desktop program using Tauri.

**Rationale:**
- **Zero Hosting Costs:** As a local application, there are no cloud servers, databases, or storage buckets to pay for.
- **Privacy & Security:** All user data and documents are stored exclusively on the user's local machine, ensuring absolute privacy.
- **Simplicity:** The application is a single executable, requiring no complex setup like DNS, SSL, or ingress tunnels.

## 2. Frontend-Backend Communication (IPC)

**Decision:** The frontend UI and Rust backend communicate via Tauri's Inter-Process Communication (IPC) commands (`invoke`).

**Rationale:**
- **Latency:** IPC is near-instantaneous and avoids the overhead of the TCP/IP stack.
- **Simplicity:** This model eliminates the need for web routers, CORS handling, or HTTP-specific middleware.
- **Security:** Tauri's Capability system provides a strict, explicit security boundary, ensuring the UI can only execute allowed native functions.

## 3. Relational Database (SQLite)

**Decision:** An embedded SQLite database is used for relational data storage.

**Rationale:**
- **No Daemon:** SQLite runs directly within the main Rust process, eliminating the need for an external database service.
- **Perfect Fit:** For a single-user desktop application, SQLite provides all necessary features without the overhead of a larger client-server database.

## 4. Background Job Processing (In-Memory Channels)

**Decision:** An in-memory channel (`tokio::sync::mpsc`) is used for managing background tasks like document processing.

**Rationale:**
- **Efficiency:** An in-memory channel is magnitudes faster and safer for a single-process application than a database-backed queue.
- **Responsiveness:** It decouples heavy background work from the UI, ensuring the application remains responsive at all times.

## 5. Document Storage (Local Filesystem)

**Decision:** Documents are stored directly on the local filesystem in the OS's standard application data directory.

**Rationale:**
- **Zero Overhead:** Avoids the complexity of managing cloud bucket credentials, regions, or network requests for local file access.
- **Native Performance:** Tauri's custom asset protocols allow the frontend to stream large files directly from the disk, offering maximum performance.

## 6. Vector Database & Pipeline (LanceDB)

**Decision:** LanceDB provides embedded vector search with serialized table creation and writes, per-document replacement, and hybrid retrieval.

**Rationale:**
- **Standalone Executable:** LanceDB runs natively inside the compiled Rust binary, requiring no external services or dependencies like Docker.
- **Replacement & Recovery:** Reindexing deletes a document's prior chunks and then merge-inserts replacements using `(document_id, page, chunk_index)` identity. SQLite is marked `indexed` and records the extraction/chunk provenance only after vector storage succeeds. A document-operations write lock serializes stale-provenance requeues with active indexing; startup and chunk-setting updates atomically return only null/mismatched indexed rows to `queued` before sending their jobs. Document deletion first changes the registry state to durable `deleting`, so retrieval excludes it until vector rows, the stored PDF, and the registry row are all removed; startup retries interrupted deletions.
- **First Access, Indexes, and Search:** A short table-creation lock prevents concurrent first upload/query callers from racing `create_empty_table`; regular reads remain concurrent after it exists. FTS is created after rows exist. The default 8-bit IVF-PQ index is deferred until LanceDB has its required 256 training vectors; smaller stores use flat vector search. LanceDB combines full-text and dense retrieval once FTS is available, and falls back to dense vector-only search with a request-scoped warning when FTS fails. Reciprocal-rank-fusion relevance drives ordering but is not presented as calibrated citation confidence.

We explicitly rejected **Chroma** for this role: it requires a Python runtime and a daemon process, directly contradicting the single-executable design tenet.

## 7. AI Model Execution & Selection

**Decision:** The application runs models locally and **in-process** by default. We use `EmbeddingGemma-300M` (via `fastembed`) for embeddings and a Qwen-class model (e.g., `Qwen3-1.7B`) loaded by **`mistral.rs`** for generation. There is no sidecar process.

**Rationale:**
- **Why `mistral.rs` over a `llama.cpp` sidecar:** For a small single-user utility, the simplicity dividend of staying in-process is enormous. Dropping the sidecar removes: PID lockfiles, ephemeral port binding, dynamic `--port` argument parsing, `GET /health` polling loops, 32-byte hex auth tokens, `Authorization: Bearer` header plumbing, `command-group`/`killpg` process-group management, `RunEvent::CloseRequested` cleanup hooks, the `shell:allow-execute` capability, the `src-tauri/binaries/` directory, per-target-triple binary naming, and a separate macOS hardened-runtime signing pass for the `llama-server` binary. We consciously accept the trade-off: `mistral.rs` (Candle-backed) does not yet match `llama.cpp`'s breadth of GPU backend coverage, particularly Vulkan on Windows. For a documentation-querying utility on commodity laptops, CPU + Metal + CUDA is sufficient and the simplicity is worth more than the long tail of GPU support.
- **Embedding Model (`EmbeddingGemma-300M`):** Available as `EmbeddingModel::EmbeddingGemma300M` via the ONNX backend in `fastembed`. It is instruction-aware (requiring `passage: ` and `query: ` prefixes), outputs 768 dimensions, and yields strong retrieval performance. Ingest splits with the active tokenizer and counts the prefix plus special tokens before embedding, preventing silent truncation. *Note:* This is distinct from `nomic-embed-text-v1.5`. `fastembed` lacks CoreML execution providers for the ONNX backend out-of-the-box, so expect CPU inference on Apple Silicon (M-series). We specifically **do not** use Candle-backed embedding models (like Qwen3) because they lack cross-platform quantization support.
- **Generation Model (`Qwen3-1.7B`):** A highly capable lightweight edge model providing excellent RAG performance within a small RAM footprint (~1.1 GB at Q4_K_M quantization), making it the ideal fast baseline for desktop hardware. Loaded via `mistral.rs`'s `GgufModelBuilder` using pre-quantized weights (Q4_K_M). Any reasoning tokens are explicitly disabled via system prompts to prevent slow overhead during structured RAG tasks.
- **No Auth Tokens Needed:** Without a localhost HTTP server in the loop, there is no surface to authenticate. The previous ephemeral-token machinery is gone entirely.
- **BYO Inference Mode:** Users can opt to use an external OpenAI-compatible API, select its `external_model` (defaulting to `gpt-4o-mini`), and store an optional API key in the OS keychain. The Remote provider dialog collects protocol, host or IP address, and optional port, composing the conventional `/v1` endpoint rather than asking users to enter an API path. There is no portable OpenAI-compatible protocol for discovering arbitrary version paths, so custom path/version probing is intentionally not attempted. The dialog tests the non-persisted composed endpoint with a supplied API-key draft or the keychain value, then lists only its returned `/models` identifiers in an app-styled dropdown. Changing any endpoint control clears the discovered list and selected model; saving endpoint and model is one transaction followed by one active-external readiness validation. Retrieval, prompt construction, and structured citations are common before the explicit `InferenceProvider` enum selects the source transport. The source-specific SSE parser and `mistral.rs` decoder emit normalized events to one request-scoped Tauri `Channel<TokenEvent>`.
- **Reranker Strategy:** We explicitly defer adding a Cross-Encoder or ColBERT reranker (e.g. `answerai-colbert-small-v1`) to the pipeline to save latency and RAM. We rely entirely on LanceDB's Hybrid Search unless user testing proves the recall is insufficient.

## 8. Source Text Extraction

**Decision:** PDFs use `pdf-extract`; DOCX files use their local ZIP/XML package structure; all other supported source files are decoded as UTF-8 text.

**Rationale:**
PDF and DOCX need format-specific extraction. PDF retains its physical page boundaries through citations. DOCX and non-paginated text do not contain trustworthy rendered page boundaries without a layout engine, so they are indexed as one logical source page and citations remain attached to the original document. The DOCX path reads `word/document.xml` with pure-Rust ZIP/XML libraries, avoiding a sidecar or cloud converter. Generic UTF-8 text support admits technical manuals, Markdown, configuration, structured data, and source files without a fragile format-by-format parser. Preview policy remains in Rust: `get_document_preview` authorizes the indexed registry record, bounds valid UTF-8 payloads, returns generic text and DOCX as escaped typed text, and parses Markdown into a constrained block/inline model. Raw HTML, images, embeds, and non-allowlisted link schemes are discarded before IPC, so Svelte never receives generated HTML.

## 9. Text Chunking Strategy

**Decision:** Tokenizer-boundary windows sized as 512 embedding tokens with an 80-token overlap.

**Rationale:** Each chunk is split and then counted using the active EmbeddingGemma tokenizer with the `passage: ` prefix and special tokens included. This makes the configured size a real embedding bound rather than a character estimate. The effective overlap is capped at half of each token window to prevent pathological near-duplicates. Every successful index records its extraction version and exact chunk pair in SQLite. Changing either setting validates the final pair, atomically requeues indexed documents whose provenance is stale, and preserves their old provenance until replacement vectors are stored successfully.

## 10. Request-Scoped Streaming

**Decision:** The application uses Tauri v2 Channels for both LLM responses (`Channel<TokenEvent>`) and document-ingestion progress (`Channel<IngestionProgress>`). Application events remain limited to lifecycle/readiness compatibility signals.

**Rationale:**
- Channels preserve ordered, request-scoped delivery where simultaneous work must not race in global listeners. Each document UI update is keyed by its document ID.
- Ingestion creates a channel-bearing queue job only for the originating upload. The worker pushes completed progress records through a separate unbounded bridge, so Tauri-channel delivery never blocks extraction, chunking, embedding, or storage. Startup/recovery jobs run without a UI channel.
- Progress values describe completed work, not estimates: a physical PDF page is reported after extraction, a nonblank page after chunking, an embedding batch after the model returns it, and `stored` only after vector and registry writes succeed.
- `mistral.rs` exposes `model.stream_chat_request(...)` as `Response::Chunk` deltas, which map to `TokenEvent::Token`. External SSE stream failures and local model stream errors are emitted as `TokenEvent::Error`; failures before a stream starts reject the `execute_query` IPC command.

## 11. Model File Sourcing

**Decision:** Model files are downloaded on first run via the `hf-hub` crate rather than bundled into the application installer.

**Rationale:**
- Avoids the massive binary size bloat (~500MB for embeddings, several GBs for LLMs).
- Allows users to seamlessly pause, resume, update, and manage their models.
- `mistral.rs` resolves tokenizer assets through Hugging Face’s cache. Ragtag owns and removes its application-local GGUF/model entries when requested, but never deletes a cache location explicitly selected through `HF_HUB_CACHE` or `HF_HOME`; that external storage is excluded from app storage accounting.

## 12. macOS Window Chrome

**Decision:** Use Tauri's native macOS overlay titlebar with hidden title text and retained traffic-light controls.

**Rationale:** Overlay removes the visually separate native title strip while keeping native close, minimize, zoom, and window-drag behavior. A compact WebView header supplies the visual bar and reserves only the space required by traffic lights, avoiding a frameless custom-control implementation.

## 13. macOS Platform Support

**Decision:** Release and bundle only Apple Silicon builds. Intel macOS exists solely as an isolated development/test path.

**Rationale:** The product does not ship an Intel target, bundled ONNX Runtime dylib, Intel entitlements, or Intel release task. The root Taskfile owns a removable external runtime at `~/.cache/mrk-ragtag/onnxruntime-1.28.0`; it rejects a runtime unless its `VERSION_NUMBER`, versioned dylib, and architecture match the pinned x86_64 ONNX Runtime 1.28.0 contract. This source/build directory stays outside the checkout, while Cargo uses its normal ignored host build output. No application business logic is Intel-specific, and the release paths remain Apple-Silicon-only.
