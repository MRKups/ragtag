//! Background document ingestion worker.
//!
//! Consumes document ingestion jobs from the queue, coordinates text extraction,
//! chunking, and embedding generation, upserts vector embeddings into LanceDB,
//! updates SQLite document records, and streams progress updates over IPC channels.

use domain::{AppError, DocumentStatus, IngestionProgress};
use log::{debug, error, info, warn};
use persistence::{AppSettingsRepository, DocumentRepository, VectorDb};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;
use tokio::sync::mpsc;

use crate::model_lifecycle::recovery::desired_indexing_provenance;
use crate::state::IngestionJob;
use persistence::storage::documents::verify_document_source_integrity;

pub const REGISTRY_READ_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
pub const REGISTRY_READ_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
pub const MAX_REGISTRY_READ_RETRIES: u32 = 5;

pub fn registry_read_retry_delay(attempt: u32) -> Duration {
    let multiplier = 1_u32 << attempt.saturating_sub(1).min(7);
    REGISTRY_READ_RETRY_BASE_DELAY
        .saturating_mul(multiplier)
        .min(REGISTRY_READ_RETRY_MAX_DELAY)
}

pub fn start_ingestion_progress_forwarder(
    channel: tauri::ipc::Channel<IngestionProgress>,
) -> mpsc::UnboundedSender<IngestionProgress> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            if channel.send(progress).is_err() {
                return;
            }
        }
    });
    sender
}

pub struct DocumentWorkerConfig {
    pub app_handle: tauri::AppHandle,
    pub pool: sqlx::SqlitePool,
    pub vector_db: Arc<VectorDb>,
    pub embedder: Arc<tokio::sync::RwLock<Option<Arc<std::sync::Mutex<fastembed::TextEmbedding>>>>>,
    pub document_operations: Arc<tokio::sync::RwLock<()>>,
    pub app_local_data: PathBuf,
    pub rx: mpsc::Receiver<IngestionJob>,
    pub tx: mpsc::Sender<IngestionJob>,
    pub active_document_id: Arc<std::sync::Mutex<Option<String>>>,
}

struct ActiveDocGuard {
    active_id: Arc<std::sync::Mutex<Option<String>>>,
}

impl Drop for ActiveDocGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_id.lock() {
            *active = None;
        }
    }
}

pub fn start_document_worker(config: DocumentWorkerConfig) {
    let DocumentWorkerConfig {
        app_handle,
        pool,
        vector_db,
        embedder: embedder_worker,
        document_operations: document_operations_worker,
        app_local_data,
        mut rx,
        tx,
        active_document_id: active_document_id_worker,
    } = config;
    let job_queue_worker = tx;
    tauri::async_runtime::spawn(async move {
        let repo = DocumentRepository::new(pool.clone());
        let mut registry_read_retries: HashMap<String, u32> = HashMap::new();
        while let Some(job) = rx.recv().await {
            let IngestionJob {
                document_id: doc_id,
                progress_channel,
            } = job;
            {
                if let Ok(mut active) = active_document_id_worker.lock() {
                    *active = Some(doc_id.clone());
                }
            }
            let _active_guard = ActiveDocGuard {
                active_id: active_document_id_worker.clone(),
            };
            let retry_progress_channel = progress_channel.clone();
            let progress_callback: Arc<dyn Fn(IngestionProgress) + Send + Sync> =
                match progress_channel.map(start_ingestion_progress_forwarder) {
                    Some(sender) => Arc::new(move |progress| {
                        let _ = sender.send(progress);
                    }),
                    None => Arc::new(|_| {}),
                };
            info!("processing document {doc_id}");

            let embedder = {
                let guard = embedder_worker.read().await;
                guard.clone()
            };
            let Some(embedder) = embedder else {
                warn!(
                    "embedder not ready — deferring document {doc_id} (will resume on next restart)"
                );
                continue;
            };
            let _document_operation = document_operations_worker.read().await;

            let result = match repo.get_by_id(&doc_id).await {
                Ok(Some(doc)) if doc.status == DocumentStatus::Queued => {
                    registry_read_retries.remove(&doc_id);
                    let settings_repo = AppSettingsRepository::new(pool.clone());
                    match desired_indexing_provenance(&settings_repo).await {
                        Ok(provenance) => {
                            let integrity = tokio::task::spawn_blocking({
                                let app_local_data = app_local_data.clone();
                                let document = doc.clone();
                                move || verify_document_source_integrity(&app_local_data, &document)
                            })
                            .await
                            .map_err(|error| {
                                AppError::internal(
                                    "Verify document integrity",
                                    format!("Document integrity task panicked: {error}"),
                                    "Restart the application and retry document ingestion.",
                                )
                            });
                            match integrity {
                                Ok(Ok(source_path)) => {
                                    rag_pipeline::process_document(
                                        rag_pipeline::DocumentIngestion {
                                            source_path: &source_path,
                                            file_type: &doc.file_type,
                                            document_id: &doc_id,
                                            provenance,
                                        },
                                        embedder,
                                        &repo,
                                        &vector_db,
                                        Arc::clone(&progress_callback),
                                    )
                                    .await
                                }
                                Ok(Err(error)) | Err(error) => Err(error),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                Ok(Some(document)) => {
                    registry_read_retries.remove(&doc_id);
                    debug!(
                        "document {doc_id} is {:?}; ignoring stale queue entry",
                        document.status,
                    );
                    continue;
                }
                Ok(None) => {
                    registry_read_retries.remove(&doc_id);
                    warn!("document {doc_id} not found in DB — may have been deleted");
                    continue;
                }
                Err(error) => {
                    let attempt = {
                        let attempt = registry_read_retries.entry(doc_id.clone()).or_insert(0);
                        *attempt = attempt.saturating_add(1);
                        *attempt
                    };
                    if attempt > MAX_REGISTRY_READ_RETRIES {
                        registry_read_retries.remove(&doc_id);
                        error!(
                            "database error while fetching document {doc_id} exceeded maximum retries ({MAX_REGISTRY_READ_RETRIES}): {error}; recording failure"
                        );
                        Err(AppError::storage(
                            "Read document registry",
                            format!(
                                "Database error while fetching document {doc_id} exceeded maximum retries ({MAX_REGISTRY_READ_RETRIES}): {error}"
                            ),
                            "Check database access permissions and restart Ragtag.",
                        ))
                    } else {
                        let delay = registry_read_retry_delay(attempt);
                        warn!(
                            "database error while fetching document {doc_id}: {error}; retrying in {delay:?} (attempt {attempt}/{MAX_REGISTRY_READ_RETRIES})"
                        );
                        let queue = job_queue_worker.clone();
                        let retry_document_id = doc_id.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(delay).await;
                            if let Err(error) = queue
                                .send(IngestionJob {
                                    document_id: retry_document_id,
                                    progress_channel: retry_progress_channel,
                                })
                                .await
                            {
                                error!("registry read retry producer stopped: {error}");
                            }
                        });
                        continue;
                    }
                }
            };

            match result {
                Ok(()) => {
                    if let Ok(Some(doc)) = repo.get_by_id(&doc_id).await {
                        if doc.status == DocumentStatus::Indexed {
                            info!("document {doc_id} indexed successfully");
                            let _ = app_handle.emit(
                                "document_indexed",
                                serde_json::json!({
                                    "document_id": &doc_id,
                                    "status": "indexed",
                                }),
                            );
                        } else {
                            debug!(
                                "document {doc_id} status is {:?}; skipping document_indexed event",
                                doc.status
                            );
                        }
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    if let Ok(Some(doc)) = repo.get_by_id(&doc_id).await
                        && doc.status == DocumentStatus::Deleting
                    {
                        debug!("document {doc_id} was deleting; skipping failure reporting");
                        continue;
                    }
                    error!("document {doc_id} processing failed: {message}");
                    let _ = repo.mark_failed(&doc_id, &message).await;
                    progress_callback(IngestionProgress::Error {
                        document_id: doc_id.clone(),
                        message: message.clone(),
                    });
                    let _ = app_handle.emit(
                        "document_failed",
                        serde_json::json!({
                            "document_id": &doc_id,
                            "error": message,
                        }),
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_registry_read_retry_delay() {
        assert_eq!(registry_read_retry_delay(1), Duration::from_millis(250));
        assert_eq!(registry_read_retry_delay(3), Duration::from_secs(1));
        assert_eq!(registry_read_retry_delay(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn declares_finite_registry_read_retries() {
        const { assert!(MAX_REGISTRY_READ_RETRIES >= 3 && MAX_REGISTRY_READ_RETRIES <= 10) };
    }
}
