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

**Decision:** LanceDB provides embedded vector search capabilities using an idempotent `merge_insert` pipeline and Hybrid Search.

**Rationale:**
- **Standalone Executable:** LanceDB runs natively inside the compiled Rust binary, requiring no external services or dependencies like Docker.
- **Idempotency & Crash Recovery:** By using `merge_insert` with a unique constraint (document ID, page, chunk), the ingestion pipeline is perfectly idempotent. If the application crashes mid-ingestion, restarting will overwrite partial data instead of appending duplicates.
- **Hybrid Search:** LanceDB natively supports combining BM25 full-text search with dense vector similarity via Reciprocal Rank Fusion (RRF), offering superior retrieval quality over vector search alone.

We explicitly rejected **Chroma** for this role: it requires a Python runtime and a daemon process, directly contradicting the single-executable design tenet.

## 7. AI Model Execution & Selection

**Decision:** The application runs models locally and **in-process** by default. We use `EmbeddingGemma-300M` (via `fastembed`) for embeddings and a Qwen-class model (e.g., `Qwen3-1.7B`) loaded by **`mistral.rs`** for generation. There is no sidecar process.

**Rationale:**
- **Why `mistral.rs` over a `llama.cpp` sidecar:** For a small single-user utility, the simplicity dividend of staying in-process is enormous. Dropping the sidecar removes: PID lockfiles, ephemeral port binding, dynamic `--port` argument parsing, `GET /health` polling loops, 32-byte hex auth tokens, `Authorization: Bearer` header plumbing, `command-group`/`killpg` process-group management, `RunEvent::CloseRequested` cleanup hooks, the `shell:allow-execute` capability, the `src-tauri/binaries/` directory, per-target-triple binary naming, and a separate macOS hardened-runtime signing pass for the `llama-server` binary. We consciously accept the trade-off: `mistral.rs` (Candle-backed) does not yet match `llama.cpp`'s breadth of GPU backend coverage, particularly Vulkan on Windows. For a documentation-querying utility on commodity laptops, CPU + Metal + CUDA is sufficient and the simplicity is worth more than the long tail of GPU support.
- **Embedding Model (`EmbeddingGemma-300M`):** Available as `EmbeddingModel::EmbeddingGemma300M` via the ONNX backend in `fastembed`. It is instruction-aware (requiring `passage: ` and `query: ` prefixes), outputs 768 dimensions, and yields strong retrieval performance. *Note:* This is distinct from `nomic-embed-text-v1.5`. `fastembed` lacks CoreML execution providers for the ONNX backend out-of-the-box, so expect CPU inference on Apple Silicon (M-series). We specifically **do not** use Candle-backed embedding models (like Qwen3) because they lack cross-platform quantization support.
- **Generation Model (`Qwen3-1.7B`):** A highly capable lightweight edge model providing excellent RAG performance within a small RAM footprint (~1.1 GB at Q4_K_M quantization), making it the ideal fast baseline for desktop hardware. Loaded via `mistral.rs`'s `GgufModelBuilder` using pre-quantized weights (Q4_K_M). Any reasoning tokens are explicitly disabled via system prompts to prevent slow overhead during structured RAG tasks.
- **No Auth Tokens Needed:** Without a localhost HTTP server in the loop, there is no surface to authenticate. The previous ephemeral-token machinery is gone entirely.
- **BYO Inference Mode:** Users can opt to use external OpenAI-compatible APIs instead of the in-process model. The external API key is securely stored in the OS keychain via the `keyring` crate. Both code paths share the same prompt-building and streaming-citation glue behind a small `InferenceClient` trait.
- **Reranker Strategy:** We explicitly defer adding a Cross-Encoder or ColBERT reranker (e.g. `answerai-colbert-small-v1`) to the pipeline to save latency and RAM. We rely entirely on LanceDB's Hybrid Search unless user testing proves the recall is insufficient.

## 8. PDF Text Extraction

**Decision:** We use `pdf-extract` (a pure Rust crate) for pulling text out of uploaded documents.

**Rationale:**
While tools like `pdfium-render` (wrapping Google's C++ Pdfium) offer superior layout fidelity, they require bundling a second OS-specific C++ shared library sidecar. To maintain our core tenet of simplicity and a pure-Rust pipeline, we accept the layout tradeoffs of `pdf-extract` for the MVP. To mitigate the impact on the citation engine, text extraction must strictly operate on a per-page loop to guarantee accurate page boundaries.

## 9. Text Chunking Strategy

**Decision:** Character-level sliding windows sized as 512 estimated tokens (about 2,048 characters), with an 80 estimated-token overlap (about 320 characters).

**Rationale:** The current ingestion path estimates one token as four characters; it does not use or claim exact model-token boundaries. This keeps chunking independent of a model-specific tokenizer while retaining useful, configurable context overlap. To prevent pathological near-duplicate chunks, the effective overlap is capped at half of each character window.

## 10. Streaming AI Responses

**Decision:** The application streams tokens from the LLM back to the frontend using Tauri v2 Channels (`Channel<TokenEvent>`).

**Rationale:**
- Channels offer ordered delivery, performance improvements, and critical backpressure compared to the older global `app.emit` event system, ensuring the UI does not hang or drop tokens during rapid text generation.
- `mistral.rs` exposes `model.stream_chat_request(...)` returning a stream of `Response::Chunk` deltas, which maps cleanly onto `TokenEvent::Token` writes into the channel — no glue code required for IPC bridging.

## 11. Model File Sourcing

**Decision:** Model files are downloaded on first run via the `hf-hub` crate rather than bundled into the application installer.

**Rationale:**
- Avoids the massive binary size bloat (~500MB for embeddings, several GBs for LLMs).
- Allows users to seamlessly pause, resume, update, and manage their models.
- `mistral.rs`'s builders resolve repo IDs against the same Hugging Face cache that `hf-hub` populates, so we control the download UX while letting the inference engine load weights normally.

## 12. macOS Window Chrome

**Decision:** Use Tauri's native macOS overlay titlebar with hidden title text and retained traffic-light controls.

**Rationale:** Overlay removes the visually separate native title strip while keeping native close, minimize, zoom, and window-drag behavior. A compact WebView header supplies the visual bar and reserves only the space required by traffic lights, avoiding a frameless custom-control implementation.
