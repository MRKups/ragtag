use domain::IngestionProgress;
use fastembed::TextEmbedding;
use persistence::VectorDb;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, RwLock, mpsc::Sender, watch};

#[derive(Default)]
pub struct LlmInitializationState {
    pub generation: u64,
    pub in_progress: bool,
    pub in_progress_generation: Option<u64>,
    pub restart_requested: bool,
}

pub struct IngestionJob {
    pub document_id: String,
    pub progress_channel: Option<tauri::ipc::Channel<IngestionProgress>>,
}

impl IngestionJob {
    pub fn background(document_id: String) -> Self {
        Self {
            document_id,
            progress_channel: None,
        }
    }
}

pub struct AppState {
    pub db_pool: SqlitePool,
    pub job_queue: Sender<IngestionJob>,
    pub embedder: Arc<RwLock<Option<Arc<StdMutex<TextEmbedding>>>>>,
    pub vector_db: Arc<VectorDb>,
    pub local_llm: Arc<RwLock<Option<Arc<mistralrs::Model>>>>,
    pub llm_initialization: Arc<Mutex<LlmInitializationState>>,
    pub registry_read_retries: Arc<Mutex<HashMap<String, u32>>>,
    pub http_client: reqwest::Client,
    pub document_operations: Arc<RwLock<()>>,
    pub active_queries: Arc<StdMutex<HashMap<String, watch::Sender<bool>>>>,
}
