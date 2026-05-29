use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    #[default]
    Queued,
    Embedding,
    Indexed,
    Failed,
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
    pub error_message: Option<String>,
    pub retry_count: i32,
}
