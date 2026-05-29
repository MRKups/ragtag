mod state;
pub use state::AppState;

use domain::{AppError, Document, DocumentStatus};
use persistence::{init_pool, DocumentRepository};
use sha2::{Digest, Sha512};
use tauri::Manager;
use tokio::sync::mpsc;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

#[tauri::command]
async fn upload_document(
    app: tauri::AppHandle,
    filename: String,
    file_bytes: Vec<u8>,
) -> Result<String, AppError> {
    let state = app.state::<AppState>();
    let repo = DocumentRepository::new(state.db_pool.clone());

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let docs_dir = app_local_data.join("documents");
    tokio::fs::create_dir_all(&docs_dir).await?;

    let hash = tokio::task::spawn_blocking({
        let bytes = file_bytes.clone();
        move || {
            let mut hasher = Sha512::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("Hashing failed: {}", e)))?;

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.pdf", &doc_id);
    let storage_path = docs_dir.join(&storage_filename);

    tokio::fs::write(&storage_path, &file_bytes).await?;

    let relative_storage_path = format!("documents/{}", storage_filename);

    // Format matches SQLite CURRENT_TIMESTAMP
    let created_at = jiff::Zoned::now()
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();

    let doc = Document {
        id: doc_id.clone(),
        filename,
        file_type: "pdf".to_string(),
        file_size_bytes: Some(file_bytes.len() as i64),
        file_hash: Some(hash),
        storage_path: relative_storage_path,
        status: DocumentStatus::Queued,
        chunk_count: None,
        embedding_model: None,
        embedding_attempts: 0,
        created_at: Some(created_at),
        processing_started_at: None,
        indexed_at: None,
        error_message: None,
        retry_count: 0,
    };

    repo.insert(&doc).await?;

    state
        .job_queue
        .try_send(doc_id.clone())
        .map_err(|_| AppError::QueueFull)?;

    Ok(doc_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))?;

            let pool = tauri::async_runtime::block_on(init_pool(&app_local_data))
                .map_err(|e| Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))?;

            let (tx, mut rx) = mpsc::channel::<String>(32);

            // Crash recovery: reset embedding → queued
            let repo = DocumentRepository::new(pool.clone());
            let recovered = tauri::async_runtime::block_on(repo.reset_embedding_to_queued())
                .map_err(|e| Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))?;
            for id in recovered {
                let _ = tx.try_send(id);
            }

            // Re-queue any documents that were 'queued' before crash
            let queued = tauri::async_runtime::block_on(repo.get_queued_ids())
                .map_err(|e| Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))?;
            for id in queued {
                let _ = tx.try_send(id);
            }

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx,
                llm_token: tokio::sync::Mutex::new(String::new()),
            });

            // Background worker: consumes jobs from the channel
            tokio::spawn(async move {
                while let Some(doc_id) = rx.recv().await {
                    eprintln!("[Worker] Received job: {}", doc_id);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ping, upload_document])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
