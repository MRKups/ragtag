use serde::{Deserialize, Serialize};

pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
pub const EXTRACTED_TEXT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Lifecycle state of a user-uploaded document in the SQLite registry (DEV-10).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    #[default]
    Queued,
    Embedding,
    Indexed,
    Deleting,
    Failed,
}

/// Ingestion settings used to index document chunks into vector storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexProvenance {
    pub extraction_version: String,
    pub chunk_size: i32,
    pub chunk_overlap: i32,
}

/// Durable document registry entity stored in SQLite (DEV-10).
///
/// Acts as the single source of truth for document identity, storage path,
/// processing lifecycle status, and indexing provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub filename: String,
    pub file_type: String,
    pub file_size_bytes: Option<i64>,
    pub file_hash: Option<Vec<u8>>,
    pub storage_path: String,
    pub status: DocumentStatus,
    pub chunk_count: Option<i32>,
    pub embedding_model: Option<String>,
    pub embedding_attempts: i32,
    pub created_at: Option<String>,
    pub processing_started_at: Option<String>,
    pub indexed_at: Option<String>,
    pub indexing_provenance: Option<IndexProvenance>,
    pub error_message: Option<String>,
    pub retry_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentPreview {
    PlainText {
        content: String,
        truncated: bool,
    },
    Markdown {
        blocks: Vec<MarkdownBlock>,
        truncated: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        content: Vec<MarkdownInline>,
    },
    Paragraph {
        content: Vec<MarkdownInline>,
    },
    BlockQuote {
        blocks: Vec<MarkdownBlock>,
    },
    List {
        start: Option<u64>,
        items: Vec<Vec<MarkdownBlock>>,
    },
    CodeBlock {
        language: Option<String>,
        content: String,
    },
    Table {
        header: Vec<Vec<MarkdownInline>>,
        rows: Vec<Vec<Vec<MarkdownInline>>>,
    },
    ThematicBreak,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarkdownInline {
    Text {
        content: String,
    },
    Code {
        content: String,
    },
    Emphasis {
        content: Vec<MarkdownInline>,
    },
    Strong {
        content: Vec<MarkdownInline>,
    },
    Strikethrough {
        content: Vec<MarkdownInline>,
    },
    Link {
        href: String,
        content: Vec<MarkdownInline>,
    },
    Break,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyStorage {
    Keychain,
    Database,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyStatus {
    pub has_key: bool,
    pub storage: ApiKeyStorage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalProviderConnection {
    pub models: Vec<String>,
}

/// Snapshot of command execution capacity across priority tiers (D018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityState {
    pub ordinary_available: usize,
    pub ordinary_capacity: usize,
    pub protected_available: usize,
    pub protected_capacity: usize,
}

/// Atomic, memory-only probe of application readiness and concurrency metrics (D018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLifecycleState {
    pub embedder_ready: bool,
    pub llm_ready: bool,
    pub inference_mode: String,
    pub model_download_active: bool,
    pub model_download_progress: Option<crate::events::ModelDownloadProgress>,
    pub active_queries_count: usize,
    pub ingestion_active: bool,
    pub active_document_id: Option<String>,
    pub capacity: CapacityState,
}
