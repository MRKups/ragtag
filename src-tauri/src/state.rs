use fastembed::TextEmbedding;
use persistence::VectorDb;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc::Sender, Mutex};

pub struct AppState {
    pub db_pool: SqlitePool,
    pub job_queue: Sender<String>,
    pub llm_token: Mutex<String>,
    pub embedder: Arc<StdMutex<TextEmbedding>>,
    pub vector_db: Arc<VectorDb>,
}
