# Ragtag: Local-First RAG Desktop App

## What This Is

A localized, zero-configuration desktop application for querying technical manuals and other documents. It acts as an intelligent, personalized brain for technical manuals, allowing users to ask questions in natural language and receive synthesized answers with precise page citations.

It is a standalone executable that processes PDFs, generates embeddings, stores data, and searches vectors entirely on the user's local machine, ensuring absolute data privacy, instantaneous file retrieval, and zero cloud hosting costs.

### Core Features

- Fully offline/local document processing and storage
- Natural language queries about your documents
- Fast, secure desktop UI powered by Tauri v2
- Instantaneous document viewing directly from the local disk
- Citation engine matching AI answers to exact PDF pages
- Background processing for document ingestion without blocking the UI
- **BYO Inference Mode:** Option to securely connect to external OpenAI-compatible endpoints instead of local models

## Tech Stack

**App Shell & UI:**
- **Framework:** Tauri v2
- **Frontend:** SvelteKit (adapter-static) / TypeScript
- **UI & Styling:** Svelte 5 (Native component-scoped CSS, no Tailwind)

**Backend Engine:**
- **Language:** Rust (Edition 2024+)
- **Async Runtime:** Tokio
- **IPC:** Tauri Commands & Events (using Tauri v2 Channels for streaming)
- **Database:** SQLite (via `sqlx`)
- **Vector DB:** LanceDB (Embedded, native Rust vector database)
- **Local Embeddings:** `fastembed-rs` (Runs ONNX models natively in Rust)
- **Local LLM (Generation):** `llama.cpp` (Managed as a secure, ephemeral Tauri Sidecar)
- **Model Downloads:** `hf-hub` (Downloads model files safely on first-run with resume support)

## How to Run

*Setup instructions will go here once the codebase is initialized.*
