use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenEvent {
    Token {
        text: String,
    },
    Warning {
        message: String,
    },
    Done {
        citations: Vec<Citation>,
    },
    Error {
        message: String,
        citations: Vec<Citation>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngestionProgress {
    Extraction {
        document_id: String,
        current_page: usize,
        total_pages: usize,
    },
    Chunking {
        document_id: String,
        current_page: i32,
        total_pages: usize,
        chunks_on_page: usize,
        total_chunks: usize,
    },
    Embedding {
        document_id: String,
        current_batch: usize,
        total_batches: usize,
        embedded_chunks: usize,
        total_chunks: usize,
    },
    Stored {
        document_id: String,
        chunk_count: usize,
    },
    Error {
        document_id: String,
        message: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Citation {
    pub document_id: String,
    pub filename: String,
    pub storage_path: String,
    pub page: i32,
    pub chunk_content: String,
    pub score: f32,
    pub is_hybrid: bool,
}
