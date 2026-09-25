use domain::{AppError, IndexProvenance};
use log::{debug, error, info, warn};
use persistence::{AppSettingsRepository, DocumentRepository, VectorDb};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::model_lifecycle::coordinator::validate_setting;
use crate::state::IngestionJob;
use crate::workers::registry_read_retry_delay;

pub const MAX_AUTOMATIC_RECOVERY_ATTEMPTS: i32 = 3;
pub const DEFAULT_CHUNK_SIZE: usize = 512;
pub const DEFAULT_CHUNK_OVERLAP: usize = 80;
pub const EXTRACTION_VERSION: &str = "token-aware-v2";

pub async fn requeue_indexed_documents_after_vector_loss(
    repo: &DocumentRepository,
) -> Result<Vec<String>, AppError> {
    repo.requeue_indexed_documents_after_vector_loss().await
}

pub fn indexed_chunk_count_mismatch(recorded_count: Option<i64>, indexed_count: Option<usize>) -> bool {
    match recorded_count.and_then(|count| usize::try_from(count).ok()) {
        Some(recorded_count) => indexed_count != Some(recorded_count),
        None => true,
    }
}

pub async fn requeue_indexed_documents_with_chunk_count_mismatches(
    repo: &DocumentRepository,
    indexed_chunk_counts: &HashMap<String, usize>,
) -> Result<Vec<String>, AppError> {
    let indexed_documents = repo.list_indexed_chunk_counts().await?;
    let ids = indexed_documents
        .into_iter()
        .filter_map(|(id, chunk_count)| {
            indexed_chunk_count_mismatch(chunk_count, indexed_chunk_counts.get(&id).copied())
                .then_some(id)
        })
        .collect::<Vec<_>>();

    repo.requeue_indexed_documents_by_ids(&ids).await?;
    Ok(ids)
}

pub fn validate_indexing_provenance(provenance: &IndexProvenance) -> Result<(), AppError> {
    if provenance.chunk_overlap >= provenance.chunk_size {
        return Err(AppError::InvalidInput(
            "chunk_overlap must be smaller than chunk_size".into(),
        ));
    }

    Ok(())
}

pub fn parse_chunk_setting(key: &str, value: &str) -> Result<i32, AppError> {
    validate_setting(key, value)?;
    value
        .parse::<i32>()
        .map_err(|_| AppError::InvalidInput(format!("Invalid {key}")))
}

pub async fn desired_indexing_provenance(
    settings: &AppSettingsRepository,
) -> Result<IndexProvenance, AppError> {
    let chunk_size = settings
        .get("chunk_size")
        .await?
        .unwrap_or_else(|| DEFAULT_CHUNK_SIZE.to_string());
    let chunk_overlap = settings
        .get("chunk_overlap")
        .await?
        .unwrap_or_else(|| DEFAULT_CHUNK_OVERLAP.to_string());
    let provenance = IndexProvenance {
        extraction_version: EXTRACTION_VERSION.to_string(),
        chunk_size: parse_chunk_setting("chunk_size", &chunk_size)?,
        chunk_overlap: parse_chunk_setting("chunk_overlap", &chunk_overlap)?,
    };
    validate_indexing_provenance(&provenance)?;
    Ok(provenance)
}

pub fn enqueue_requeued_documents(
    queue: mpsc::Sender<IngestionJob>,
    document_ids: Vec<String>,
) -> Result<(), AppError> {
    let mut remaining = document_ids.into_iter();

    while let Some(document_id) = remaining.next() {
        match queue.try_send(IngestionJob::background(document_id)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(job)) => {
                tauri::async_runtime::spawn(async move {
                    for job in std::iter::once(job).chain(remaining.map(IngestionJob::background)) {
                        if let Err(error) = queue.send(job).await {
                            error!(
                                "stale document reindex recovery producer stopped before enqueue: {error}"
                            );
                            break;
                        }
                    }
                });
                return Err(AppError::QueueFull);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                return Err(AppError::Internal(
                    "document worker stopped before stale document reindex could be queued".into(),
                ));
            }
        }
    }

    Ok(())
}

pub async fn recover_startup_documents(
    pool: &sqlx::SqlitePool,
    vector_db: &VectorDb,
    document_operations: &tokio::sync::RwLock<()>,
) -> Result<HashSet<String>, AppError> {
    let repo = DocumentRepository::new(pool.clone());
    let _document_operation = document_operations.write().await;
    let mut recovered_ids = HashSet::new();

    if !vector_db.chunks_table_exists().await? {
        recovered_ids.extend(requeue_indexed_documents_after_vector_loss(&repo).await?);
    } else {
        let indexed_chunk_counts = vector_db.indexed_chunk_counts().await?;
        recovered_ids.extend(
            requeue_indexed_documents_with_chunk_count_mismatches(&repo, &indexed_chunk_counts)
                .await?,
        );
    }

    let settings = AppSettingsRepository::new(pool.clone());
    let desired_provenance = desired_indexing_provenance(&settings).await?;
    recovered_ids.extend(
        repo.requeue_indexed_with_stale_provenance(&desired_provenance)
            .await?,
    );
    recovered_ids.extend(
        repo.reset_interrupted_embeddings(MAX_AUTOMATIC_RECOVERY_ATTEMPTS)
            .await?,
    );
    recovered_ids.extend(
        repo.reset_failed_to_queued(MAX_AUTOMATIC_RECOVERY_ATTEMPTS)
            .await?,
    );
    recovered_ids.extend(repo.get_queued_ids().await?);
    Ok(recovered_ids)
}

pub async fn recover_startup_documents_with_retry(
    pool: sqlx::SqlitePool,
    vector_db: Arc<VectorDb>,
    document_operations: Arc<tokio::sync::RwLock<()>>,
    queue: mpsc::Sender<IngestionJob>,
) {
    let mut attempt = 0_u32;
    loop {
        match recover_startup_documents(&pool, &vector_db, &document_operations).await {
            Ok(recovered_ids) => {
                let recovered_count = recovered_ids.len();
                for id in recovered_ids {
                    if let Err(error) = queue.send(IngestionJob::background(id)).await {
                        error!(
                            "startup recovery — document worker stopped before recovery completed: {error}"
                        );
                        return;
                    }
                }
                if recovered_count > 0 {
                    info!("startup recovery — re-queued {recovered_count} documents");
                } else {
                    debug!("startup recovery — no documents to resume");
                }
                return;
            }
            Err(error) => {
                attempt = attempt.saturating_add(1);
                let delay = registry_read_retry_delay(attempt);
                warn!(
                    "startup recovery — failed to read durable document work: {error}; retrying in {delay:?}"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Document, DocumentStatus};
    use std::time::Duration;
    use tokio::time::timeout;

    fn indexed_text_document(filename: &str) -> Document {
        Document {
            id: "document-1".into(),
            filename: filename.into(),
            file_type: "text".into(),
            file_size_bytes: Some(10),
            file_hash: None,
            storage_path: "documents/document-1.txt".into(),
            status: DocumentStatus::Indexed,
            chunk_count: Some(1),
            embedding_model: None,
            embedding_attempts: 1,
            created_at: None,
            processing_started_at: None,
            indexed_at: None,
            indexing_provenance: None,
            error_message: None,
            retry_count: 0,
        }
    }

    #[test]
    fn detects_indexed_chunk_count_mismatches() {
        assert!(!indexed_chunk_count_mismatch(Some(2), Some(2)));
        assert!(indexed_chunk_count_mismatch(Some(2), Some(1)));
        assert!(indexed_chunk_count_mismatch(None, Some(0)));
        assert!(indexed_chunk_count_mismatch(Some(-1), None));
    }

    #[tokio::test]
    async fn requeues_indexed_documents_when_the_chunks_table_is_missing() {
        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-lost-vectors-{}", uuid::Uuid::now_v7()));
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let repo = persistence::DocumentRepository::new(pool.clone());
        let mut document = indexed_text_document("lost-vectors.txt");
        document.embedding_attempts = 3;
        repo.insert(&document).await.unwrap();
        let vector_db = persistence::VectorDb::init(&app_local_data.join(".lancedb"))
            .await
            .unwrap();
        let document_operations = tokio::sync::RwLock::new(());

        let recovered = recover_startup_documents(&pool, &vector_db, &document_operations)
            .await
            .unwrap();

        assert!(recovered.contains(&document.id));
        assert_eq!(
            repo.get_by_id(&document.id).await.unwrap().unwrap().status,
            DocumentStatus::Queued
        );
        assert_eq!(
            repo.get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .embedding_attempts,
            0
        );
        drop(vector_db);
        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[tokio::test]
    async fn startup_recovery_caps_interrupted_embedding_attempts() {
        let app_local_data = std::env::temp_dir().join(format!(
            "ragtag-interrupted-embeddings-{}",
            uuid::Uuid::now_v7()
        ));
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let repo = persistence::DocumentRepository::new(pool.clone());

        let mut retryable = indexed_text_document("retryable.txt");
        retryable.id = "retryable".into();
        retryable.status = DocumentStatus::Embedding;
        retryable.embedding_attempts = MAX_AUTOMATIC_RECOVERY_ATTEMPTS - 1;
        repo.insert(&retryable).await.unwrap();

        let mut exhausted = indexed_text_document("exhausted.txt");
        exhausted.id = "exhausted".into();
        exhausted.status = DocumentStatus::Embedding;
        exhausted.embedding_attempts = MAX_AUTOMATIC_RECOVERY_ATTEMPTS;
        repo.insert(&exhausted).await.unwrap();

        let vector_db = persistence::VectorDb::init(&app_local_data.join(".lancedb"))
            .await
            .unwrap();
        let document_operations = tokio::sync::RwLock::new(());

        let recovered = recover_startup_documents(&pool, &vector_db, &document_operations)
            .await
            .unwrap();

        assert!(recovered.contains(&retryable.id));
        assert!(!recovered.contains(&exhausted.id));
        assert_eq!(
            repo.get_by_id(&retryable.id).await.unwrap().unwrap().status,
            DocumentStatus::Queued
        );
        assert_eq!(
            repo.get_by_id(&exhausted.id).await.unwrap().unwrap().status,
            DocumentStatus::Failed
        );

        drop(vector_db);
        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[tokio::test]
    async fn reindex_queue_overflow_returns_without_blocking_and_recovers_in_background() {
        let (queue, mut receiver) = mpsc::channel(1);
        queue
            .try_send(IngestionJob::background("already-queued".to_string()))
            .unwrap();

        let error = enqueue_requeued_documents(
            queue.clone(),
            vec!["first-stale".to_string(), "second-stale".to_string()],
        )
        .unwrap_err();
        assert!(matches!(error, AppError::QueueFull));
        assert_eq!(
            receiver.recv().await.map(|job| job.document_id).as_deref(),
            Some("already-queued")
        );
        assert_eq!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .map(|job| job.document_id)
                .as_deref(),
            Some("first-stale")
        );
        assert_eq!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .map(|job| job.document_id)
                .as_deref(),
            Some("second-stale")
        );
    }
}
