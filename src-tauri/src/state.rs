use fastembed::TextEmbedding;
use persistence::VectorDb;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc::Sender, watch, RwLock};

pub struct AppState {
    pub db_pool: SqlitePool,
    pub job_queue: Sender<String>,
    pub embedder: Arc<RwLock<Option<Arc<StdMutex<TextEmbedding>>>>>,
    pub vector_db: Arc<VectorDb>,
    pub local_llm: Arc<RwLock<Option<Arc<mistralrs::Model>>>>,
    pub http_client: reqwest::Client,
    pub document_operations: Arc<RwLock<()>>,
    pub active_queries: Arc<StdMutex<HashMap<String, watch::Sender<bool>>>>,
}
