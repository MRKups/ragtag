mod commands;
mod inference;
mod model_download;
mod model_lifecycle;
mod state;
mod storage;
mod workers;

use commands::*;
use model_lifecycle::*;
pub use state::{AppState, IngestionJob};
use storage::*;
use workers::*;

use log::{error, info};
use persistence::{DocumentRepository, VectorDb, init_pool};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::mpsc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .setup(|app| {
            let app_local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            // Complete a requested reset before opening either data store. On any
            // failure the sentinel remains in place so the next launch retries it.
            let sentinel = app_local_data.join(".reset_pending");
            if let Err(error) = complete_pending_reset(&app_local_data, &sentinel) {
                error!("pending data reset failed: {error}");
                return Err(Box::new(std::io::Error::other(error.to_string())));
            }

            let pool = tauri::async_runtime::block_on(init_pool(&app_local_data))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            let embedder: Arc<tokio::sync::RwLock<Option<Arc<std::sync::Mutex<fastembed::TextEmbedding>>>>> =
                Arc::new(tokio::sync::RwLock::new(None));

            let (tx, rx) = mpsc::channel::<IngestionJob>(32);

            let lancedb_dir = app_local_data.join(".lancedb");
            let vector_db = tauri::async_runtime::block_on(VectorDb::init(&lancedb_dir))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            let vector_db = Arc::new(vector_db);
            let chunks_table_exists =
                tauri::async_runtime::block_on(vector_db.chunks_table_exists()).map_err(|error| {
                    Box::new(std::io::Error::other(error.to_string()))
                })?;
            let recovery_repo = DocumentRepository::new(pool.clone());
            if !chunks_table_exists {
                let requeued = tauri::async_runtime::block_on(
                    requeue_indexed_documents_after_vector_loss(&recovery_repo),
                )
                .map_err(|error| Box::new(std::io::Error::other(error.to_string())))?;
                if !requeued.is_empty() {
                    info!(
                        "startup vector recovery — re-queued {} indexed documents after chunks table loss",
                        requeued.len()
                    );
                }
            } else {
                let indexed_chunk_counts =
                    tauri::async_runtime::block_on(vector_db.indexed_chunk_counts()).map_err(
                        |error| Box::new(std::io::Error::other(error.to_string())),
                    )?;
                let requeued = tauri::async_runtime::block_on(
                    requeue_indexed_documents_with_chunk_count_mismatches(
                        &recovery_repo,
                        &indexed_chunk_counts,
                    ),
                )
                .map_err(|error| Box::new(std::io::Error::other(error.to_string())))?;
                if !requeued.is_empty() {
                    info!(
                        "startup vector recovery — re-queued {} indexed documents with mismatched chunk counts",
                        requeued.len()
                    );
                }
            }
            let deletion_repo = DocumentRepository::new(pool.clone());
            tauri::async_runtime::block_on(reconcile_pending_deletions(
                &deletion_repo,
                &vector_db,
                &app_local_data,
            ));

            match tauri::async_runtime::block_on(reconcile_orphan_document_sources(
                &deletion_repo,
                &app_local_data,
            )) {
                Ok(0) => {}
                Ok(removed) => {
                    info!("startup source reconciliation — removed {removed} orphaned source files");
                }
                Err(error) => {
                    error!("startup source reconciliation — failed: {error}");
                }
            }

            let local_llm = Arc::new(tokio::sync::RwLock::new(None));
            let llm_initialization = Arc::new(tokio::sync::Mutex::new(
                state::LlmInitializationState::default(),
            ));
            let document_operations = Arc::new(tokio::sync::RwLock::new(()));
            let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx.clone(),
                embedder: embedder.clone(),
                vector_db: vector_db.clone(),
                local_llm: local_llm.clone(),
                llm_initialization,
                document_operations: document_operations.clone(),
                active_queries: active_queries.clone(),
                http_client: reqwest::Client::builder()
                    .use_rustls_tls()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| Box::new(std::io::Error::other(e.to_string())))?,
            });

            // Background worker: processes documents from the channel
            start_document_worker(DocumentWorkerConfig {
                app_handle: app.handle().clone(),
                pool,
                vector_db,
                embedder,
                document_operations,
                docs_dir: app_local_data,
                rx,
                tx,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            begin_first_run_init,
            retry_local_model_initialization,
            upload_document,
            execute_query,
            cancel_query,
            list_documents,
            get_document_preview,
            delete_document,
            get_settings,
            set_setting,
            set_api_key,
            test_external_provider,
            set_external_provider_config,
            get_api_key_status,
            delete_api_key,
            get_storage_info,
            compact_vector_store,
            vacuum_database,
            reset_all_data,
            get_platform
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
