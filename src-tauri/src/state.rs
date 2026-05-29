use sqlx::SqlitePool;
use tokio::sync::{mpsc::Sender, Mutex};

pub struct AppState {
    pub db_pool: SqlitePool,
    pub job_queue: Sender<String>,
    pub llm_token: Mutex<String>,
}
