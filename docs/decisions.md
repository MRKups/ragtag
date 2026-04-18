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

## 7. AI Model Execution & Selection

**Decision:** The application runs models locally by default. We use `EmbeddingGemma-300M` for embeddings and `Gemma 4 E2B` for generation.

**Rationale:**
- **The Sidecar Decision:** `llama.cpp` (`llama-server`) is adopted as the single justified sidecar. In 2026, no pure-Rust alternative provides the necessary Vulkan/GPU acceleration coverage across platforms. By contrast, `fastembed-rs` runs in-process perfectly for embeddings.
- **Windows GPU Backend:** We ship the Vulkan + CPU binary as the default baseline to maximize compatibility (NVIDIA, AMD, Intel). An optional CUDA download pack may be provided post-launch if required.
- **Embedding Model (`EmbeddingGemma-300M`):** Replaces older models like MiniLM. Available as `EmbeddingModel::EmbeddingGemma300M` via the ONNX backend in `fastembed`. It is instruction-aware (requiring `passage: ` and `query: ` prefixes), outputs 768 dimensions, and yields vastly superior retrieval performance. *Note:* This is distinct from `nomic-embed-text-v1.5`. Furthermore, `fastembed` lacks CoreML execution providers for the ONNX backend out-of-the-box, so expect CPU inference on Apple Silicon (M-series). We specifically **do not** use Candle-backed models (like Qwen3) because they lack cross-platform GPU/quantization support.
- **Generation Model (`Gemma 4 E2B`):** Released in 2025, this edge model offers a massive 128K context window, Apache 2.0 licensing, and excellent RAG performance within a ~5 GB RAM footprint (at Q4 quantization), making it the ideal baseline for desktop hardware. The `<|think|>` token must be explicitly disabled via system prompts to prevent slow reasoning overhead during structured RAG tasks.
- **Ephemeral API Token:** For local sidecar security, Rust generates a 32-byte cryptographically random hex token on startup. The sidecar binds dynamically to an ephemeral localhost port and requires this token.
- **BYO Inference Mode:** Users can opt to use external APIs instead of the local sidecar. The external API key is securely stored in the OS keychain via the `keyring` crate.
- **Reranker Strategy:** We explicitly defer adding a Cross-Encoder or ColBERT reranker (e.g. `answerai-colbert-small-v1`) to the pipeline to save latency and RAM. We rely entirely on LanceDB's Hybrid Search unless user testing proves the recall is insufficient.

## 8. PDF Text Extraction

**Decision:** We use `pdf-extract` (a pure Rust crate) for pulling text out of uploaded documents.

**Rationale:**
While tools like `pdfium-render` (wrapping Google's C++ Pdfium) offer superior layout fidelity, they require bundling a second OS-specific C++ shared library sidecar. To maintain our core tenet of simplicity and a pure-Rust pipeline, we accept the layout tradeoffs of `pdf-extract` for the MVP. To mitigate the impact on the citation engine, text extraction must strictly operate on a per-page loop to guarantee accurate page boundaries.

## 8. Text Chunking Strategy

**Decision:** 512 tokens per chunk with an 80-token overlap.

**Rationale:** Token-based chunking balances context retention with LLM context windows. 512 tokens provides enough density for semantic meaning without diluting the chunk. The 80-token overlap ensures concepts crossing chunk boundaries are not lost during retrieval.

## 9. Streaming AI Responses

**Decision:** The application streams tokens from the LLM back to the frontend using Tauri v2 Channels (`Channel<TokenEvent>`).

**Rationale:**
- Channels offer ordered delivery, performance improvements, and critical backpressure compared to the older global `app.emit` event system, ensuring the UI does not hang or drop tokens during rapid text generation.

## 10. Model File Sourcing

**Decision:** Model files are downloaded on first run via the `hf-hub` crate rather than bundled into the application installer.

**Rationale:**
- Avoids the massive binary size bloat (~45MB for embeddings, several GBs for LLMs).
- Allows users to seamlessly pause, resume, update, and manage their models.
