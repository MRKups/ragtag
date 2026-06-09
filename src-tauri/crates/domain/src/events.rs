use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenEvent {
    Token { text: String },
    Done { citations: Vec<Citation> },
    Error { message: String, citations: Vec<Citation> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Citation {
    pub document_id: String,
    pub filename: String,
    pub storage_path: String,
    pub page: i32,
    pub chunk_content: String,
    pub score: f32,
}
