use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexProvenance {
    pub extraction_version: String,
    pub chunk_size: i32,
    pub chunk_overlap: i32,
}

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
