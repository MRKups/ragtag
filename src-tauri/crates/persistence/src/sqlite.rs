use domain::{AppError, Document, DocumentStatus, models::IndexProvenance};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::path::Path;

const DOCUMENT_SELECT: &str = "SELECT
    id, filename, file_type, file_size_bytes, file_hash, storage_path, status,
    chunk_count, embedding_model, embedding_attempts,
    CAST(created_at AS TEXT) AS created_at,
    CAST(processing_started_at AS TEXT) AS processing_started_at,
    CAST(indexed_at AS TEXT) AS indexed_at,
    indexed_extraction_version, indexed_chunk_size, indexed_chunk_overlap,
    error_message, retry_count
    FROM documents";

// Retain successful indexing provenance until replacement indexing succeeds.
const INDEXED_RESET: &str = "UPDATE documents
    SET status = 'queued',
        chunk_count = NULL,
        embedding_model = NULL,
        embedding_attempts = 0,
        processing_started_at = NULL,
        indexed_at = NULL,
        error_message = NULL,
        retry_count = 0";

const STALE_INDEXED_PREDICATE: &str = "status = 'indexed'
    AND (
        indexed_extraction_version IS NULL
        OR indexed_extraction_version != ?
        OR indexed_chunk_size IS NULL
        OR indexed_chunk_size != ?
        OR indexed_chunk_overlap IS NULL
        OR indexed_chunk_overlap != ?
    )";

fn sql_err(e: sqlx::Error) -> AppError {
    AppError::storage(
        "Database operation",
        e.to_string(),
        "Check that the application data directory is writable and restart Ragtag.",
    )
}

fn insert_document_err(error: sqlx::Error) -> AppError {
    if error.as_database_error().is_some_and(|database_error| {
        database_error
            .message()
            .contains("UNIQUE constraint failed: documents.file_hash")
    }) {
        AppError::duplicate_document(
            "Register document",
            "This file was uploaded concurrently",
            "Verify whether the document is already present in your document library.",
        )
    } else {
        sql_err(error)
    }
}

pub async fn init_pool(app_local_data_dir: &Path) -> Result<SqlitePool, AppError> {
    std::fs::create_dir_all(app_local_data_dir)?;

    let db_path = app_local_data_dir.join("app.db");

    let options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(sql_err)?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            AppError::storage(
                "Run database migrations",
                format!("Migration failed: {e}"),
                "Verify database file permissions and restart Ragtag.",
            )
        })?;

    Ok(pool)
}

pub async fn vacuum(pool: &SqlitePool) -> Result<(), AppError> {
    sqlx::query("VACUUM").execute(pool).await.map_err(sql_err)?;
    Ok(())
}

pub struct DocumentRepository {
    pool: SqlitePool,
}

impl DocumentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, doc: &Document) -> Result<(), AppError> {
        let status_str = match doc.status {
            DocumentStatus::Queued => "queued",
            DocumentStatus::Embedding => "embedding",
            DocumentStatus::Indexed => "indexed",
            DocumentStatus::Deleting => "deleting",
            DocumentStatus::Failed => "failed",
        };

        sqlx::query(
            r#"
            INSERT INTO documents (
                id, filename, file_type, file_size_bytes, file_hash, storage_path,
                status, chunk_count, embedding_model, embedding_attempts,
                created_at, processing_started_at, indexed_at,
                indexed_extraction_version, indexed_chunk_size, indexed_chunk_overlap,
                error_message, retry_count
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&doc.id)
        .bind(&doc.filename)
        .bind(&doc.file_type)
        .bind(doc.file_size_bytes)
        .bind(doc.file_hash.as_deref())
        .bind(&doc.storage_path)
        .bind(status_str)
        .bind(doc.chunk_count)
        .bind(doc.embedding_model.as_deref())
        .bind(doc.embedding_attempts)
        .bind(doc.created_at.as_deref())
        .bind(doc.processing_started_at.as_deref())
        .bind(doc.indexed_at.as_deref())
        .bind(
            doc.indexing_provenance
                .as_ref()
                .map(|provenance| provenance.extraction_version.as_str()),
        )
        .bind(
            doc.indexing_provenance
                .as_ref()
                .map(|provenance| provenance.chunk_size),
        )
        .bind(
            doc.indexing_provenance
                .as_ref()
                .map(|provenance| provenance.chunk_overlap),
        )
        .bind(doc.error_message.as_deref())
        .bind(doc.retry_count)
        .execute(&self.pool)
        .await
        .map_err(insert_document_err)?;

        Ok(())
    }

    pub async fn mark_embedding(&self, id: &str) -> Result<bool, AppError> {
        let result = sqlx::query(
            "UPDATE documents
             SET status = 'embedding',
                 processing_started_at = CURRENT_TIMESTAMP,
                 embedding_attempts = COALESCE(embedding_attempts, 0) + 1,
                 error_message = NULL
             WHERE id = ? AND status != 'deleting'",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_indexed(
        &self,
        id: &str,
        chunk_count: usize,
        embedding_model: &str,
        provenance: &IndexProvenance,
    ) -> Result<bool, AppError> {
        let chunk_count = i32::try_from(chunk_count).map_err(|_| {
            AppError::storage(
                "Store document chunk count",
                "Chunk count exceeds SQLite integer range",
                "Split the document into smaller sections under the chunk budget.",
            )
        })?;

        let result = sqlx::query(
            "UPDATE documents
             SET status = 'indexed',
                 chunk_count = ?,
                 embedding_model = ?,
                 embedding_attempts = 0,
                 indexed_at = CURRENT_TIMESTAMP,
                 indexed_extraction_version = ?,
                 indexed_chunk_size = ?,
                 indexed_chunk_overlap = ?,
                 error_message = NULL
             WHERE id = ? AND status = 'embedding'",
        )
        .bind(chunk_count)
        .bind(embedding_model)
        .bind(&provenance.extraction_version)
        .bind(provenance.chunk_size)
        .bind(provenance.chunk_overlap)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<Document>, AppError> {
        let mut query = sqlx::QueryBuilder::new(DOCUMENT_SELECT);
        let row = query
            .push(" WHERE id = ")
            .push_bind(id)
            .build_query_as::<DocumentRow>()
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(row.map(DocumentRow::into_domain))
    }

    pub async fn get_deleting_documents(&self) -> Result<Vec<Document>, AppError> {
        Ok(self
            .list_all()
            .await?
            .into_iter()
            .filter(|document| document.status == DocumentStatus::Deleting)
            .collect())
    }

    pub async fn get_queued_ids(&self) -> Result<Vec<String>, AppError> {
        let rows = sqlx::query!("SELECT id FROM documents WHERE status = 'queued'")
            .fetch_all(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(rows.into_iter().filter_map(|r| r.id).collect())
    }

    pub async fn get_indexed_ids(&self) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar("SELECT id FROM documents WHERE status = 'indexed'")
            .fetch_all(&self.pool)
            .await
            .map_err(sql_err)
    }

    pub async fn requeue_indexed_with_stale_provenance(
        &self,
        desired_provenance: &IndexProvenance,
    ) -> Result<Vec<String>, AppError> {
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;
        let ids = requeue_stale_indexed_documents(&mut transaction, desired_provenance).await?;
        transaction.commit().await.map_err(sql_err)?;
        Ok(ids)
    }

    pub async fn list_all(&self) -> Result<Vec<Document>, AppError> {
        let mut query = sqlx::QueryBuilder::new(DOCUMENT_SELECT);
        let rows = query
            .push(" ORDER BY created_at DESC")
            .build_query_as::<DocumentRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(rows.into_iter().map(DocumentRow::into_domain).collect())
    }

    pub async fn mark_failed(&self, id: &str, error_message: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE documents SET status = 'failed', retry_count = retry_count + 1, error_message = ? WHERE id = ? AND status != 'deleting'"
        )
        .bind(error_message)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn mark_deleting(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE documents SET status = 'deleting', error_message = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(())
    }

    pub async fn find_by_hash(&self, hash: &[u8]) -> Result<Option<Document>, AppError> {
        let mut query = sqlx::QueryBuilder::new(DOCUMENT_SELECT);
        let row = query
            .push(" WHERE file_hash = ")
            .push_bind(hash)
            .build_query_as::<DocumentRow>()
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(row.map(DocumentRow::into_domain))
    }

    pub async fn delete_by_id(&self, id: &str) -> Result<(), AppError> {
        sqlx::query!("DELETE FROM documents WHERE id = ?", id)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        Ok(())
    }

    pub async fn reset_interrupted_embeddings(
        &self,
        max_attempts: i32,
    ) -> Result<Vec<String>, AppError> {
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;
        let ids = sqlx::query_scalar(
            "SELECT id
             FROM documents
             WHERE status = 'embedding' AND COALESCE(embedding_attempts, 0) < ?",
        )
        .bind(max_attempts)
        .fetch_all(&mut *transaction)
        .await
        .map_err(sql_err)?;

        if !ids.is_empty() {
            sqlx::query(
                "UPDATE documents
                 SET status = 'queued',
                     processing_started_at = NULL,
                     error_message = NULL
                 WHERE status = 'embedding' AND COALESCE(embedding_attempts, 0) < ?",
            )
            .bind(max_attempts)
            .execute(&mut *transaction)
            .await
            .map_err(sql_err)?;
        }

        sqlx::query(
            "UPDATE documents
             SET status = 'failed',
                 processing_started_at = NULL,
                 retry_count = MAX(COALESCE(retry_count, 0), ?),
                 error_message = 'Document processing reached the automatic recovery limit. Remove it and upload again to retry.'
             WHERE status = 'embedding' AND COALESCE(embedding_attempts, 0) >= ?",
        )
        .bind(max_attempts)
        .bind(max_attempts)
        .execute(&mut *transaction)
        .await
        .map_err(sql_err)?;

        transaction.commit().await.map_err(sql_err)?;
        Ok(ids)
    }

    pub async fn reset_failed_to_queued(&self, max_retries: i32) -> Result<Vec<String>, AppError> {
        let docs = sqlx::query!(
            "SELECT id FROM documents WHERE status = 'failed' AND retry_count < ?",
            max_retries
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;

        let ids: Vec<String> = docs.into_iter().filter_map(|d| d.id).collect();

        if !ids.is_empty() {
            sqlx::query!(
                "UPDATE documents SET status = 'queued' WHERE status = 'failed' AND retry_count < ?",
                max_retries
            )
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        }

        Ok(ids)
    }

    pub async fn requeue_indexed_documents_after_vector_loss(
        &self,
    ) -> Result<Vec<String>, AppError> {
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;
        let ids =
            sqlx::query_scalar::<_, String>("SELECT id FROM documents WHERE status = 'indexed'")
                .fetch_all(&mut *transaction)
                .await
                .map_err(sql_err)?;
        if !ids.is_empty() {
            let mut query = sqlx::QueryBuilder::new(INDEXED_RESET);
            query
                .push(" WHERE status = 'indexed'")
                .build()
                .execute(&mut *transaction)
                .await
                .map_err(sql_err)?;
        }
        transaction.commit().await.map_err(sql_err)?;
        Ok(ids)
    }

    pub async fn list_indexed_chunk_counts(&self) -> Result<Vec<(String, Option<i64>)>, AppError> {
        sqlx::query_as::<_, (String, Option<i64>)>(
            "SELECT id, chunk_count FROM documents WHERE status = 'indexed'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)
    }

    pub async fn requeue_indexed_documents_by_ids(&self, ids: &[String]) -> Result<(), AppError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;
        for id in ids {
            let mut query = sqlx::QueryBuilder::new(INDEXED_RESET);
            query
                .push(" WHERE id = ")
                .push_bind(id)
                .push(" AND status = 'indexed'")
                .build()
                .execute(&mut *transaction)
                .await
                .map_err(sql_err)?;
        }
        transaction.commit().await.map_err(sql_err)?;
        Ok(())
    }
}

async fn requeue_stale_indexed_documents(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    desired_provenance: &IndexProvenance,
) -> Result<Vec<String>, AppError> {
    let mut select = sqlx::QueryBuilder::new("SELECT id FROM documents WHERE ");
    let ids = select
        .push(STALE_INDEXED_PREDICATE)
        .build_query_scalar::<String>()
        .bind(&desired_provenance.extraction_version)
        .bind(desired_provenance.chunk_size)
        .bind(desired_provenance.chunk_overlap)
        .fetch_all(&mut **transaction)
        .await
        .map_err(sql_err)?;

    if !ids.is_empty() {
        let mut update = sqlx::QueryBuilder::new(INDEXED_RESET);
        update
            .push(" WHERE ")
            .push(STALE_INDEXED_PREDICATE)
            .build()
            .bind(&desired_provenance.extraction_version)
            .bind(desired_provenance.chunk_size)
            .bind(desired_provenance.chunk_overlap)
            .execute(&mut **transaction)
            .await
            .map_err(sql_err)?;
    }

    Ok(ids)
}

async fn upsert_setting_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO app_settings (key, value)
         VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **transaction)
    .await
    .map_err(sql_err)?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    filename: String,
    file_type: String,
    file_size_bytes: Option<i64>,
    file_hash: Option<Vec<u8>>,
    storage_path: String,
    status: String,
    chunk_count: Option<i64>,
    embedding_model: Option<String>,
    embedding_attempts: Option<i64>,
    created_at: Option<String>,
    processing_started_at: Option<String>,
    indexed_at: Option<String>,
    indexed_extraction_version: Option<String>,
    indexed_chunk_size: Option<i64>,
    indexed_chunk_overlap: Option<i64>,
    error_message: Option<String>,
    retry_count: Option<i64>,
}

impl DocumentRow {
    fn into_domain(self) -> Document {
        let status = match self.status.as_str() {
            "embedding" => DocumentStatus::Embedding,
            "indexed" => DocumentStatus::Indexed,
            "deleting" => DocumentStatus::Deleting,
            "failed" => DocumentStatus::Failed,
            _ => DocumentStatus::Queued,
        };

        let indexing_provenance = match (
            self.indexed_extraction_version,
            self.indexed_chunk_size,
            self.indexed_chunk_overlap,
        ) {
            (Some(extraction_version), Some(chunk_size), Some(chunk_overlap)) => {
                match (i32::try_from(chunk_size), i32::try_from(chunk_overlap)) {
                    (Ok(chunk_size), Ok(chunk_overlap)) => Some(IndexProvenance {
                        extraction_version,
                        chunk_size,
                        chunk_overlap,
                    }),
                    _ => None,
                }
            }
            _ => None,
        };

        Document {
            id: self.id,
            filename: self.filename,
            file_type: self.file_type,
            file_size_bytes: self.file_size_bytes,
            file_hash: self.file_hash,
            storage_path: self.storage_path,
            status,
            chunk_count: self.chunk_count.map(|c| c as i32),
            embedding_model: self.embedding_model,
            embedding_attempts: self.embedding_attempts.unwrap_or(0) as i32,
            created_at: self.created_at,
            processing_started_at: self.processing_started_at,
            indexed_at: self.indexed_at,
            indexing_provenance,
            error_message: self.error_message,
            retry_count: self.retry_count.unwrap_or(0) as i32,
        }
    }
}

pub struct AppSettingsRepository {
    pool: SqlitePool,
}

impl AppSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, AppError> {
        let row = sqlx::query!(r#"SELECT value FROM app_settings WHERE key = ?"#, key)
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(row.map(|r| r.value))
    }

    pub async fn get_all(&self) -> Result<std::collections::HashMap<String, String>, AppError> {
        let rows = sqlx::query_as::<_, (String, String)>("SELECT key, value FROM app_settings")
            .fetch_all(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(rows.into_iter().collect())
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
        // Keep this literal independently checked by SQLx's offline metadata.
        sqlx::query!(
            r#"
            INSERT INTO app_settings (key, value)
            VALUES (?, ?)
            ON CONFLICT(key) DO UPDATE SET 
                value = excluded.value
            "#,
            key,
            value
        )
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<bool, AppError> {
        let deleted = sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;

        Ok(deleted.rows_affected() != 0)
    }

    pub async fn set_external_inference_config(
        &self,
        base_url: &str,
        model: &str,
    ) -> Result<(), AppError> {
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;

        upsert_setting_in_transaction(&mut transaction, "external_base_url", base_url).await?;
        upsert_setting_in_transaction(&mut transaction, "external_model", model).await?;

        transaction.commit().await.map_err(sql_err)?;
        Ok(())
    }

    pub async fn set_indexing_setting_and_requeue_stale_documents(
        &self,
        key: &str,
        value: &str,
        desired_provenance: &IndexProvenance,
    ) -> Result<Vec<String>, AppError> {
        let mut transaction = self.pool.begin().await.map_err(sql_err)?;
        upsert_setting_in_transaction(&mut transaction, key, value).await?;

        let ids = requeue_stale_indexed_documents(&mut transaction, desired_provenance).await?;
        transaction.commit().await.map_err(sql_err)?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppSettingsRepository, DocumentRepository, init_pool};
    use domain::{Document, DocumentStatus, models::IndexProvenance};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DATABASE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn indexed_document(id: &str, provenance: IndexProvenance) -> Document {
        Document {
            id: id.to_string(),
            filename: format!("{id}.pdf"),
            file_type: "application/pdf".to_string(),
            file_size_bytes: Some(1),
            file_hash: None,
            storage_path: format!("{id}.pdf"),
            status: DocumentStatus::Indexed,
            chunk_count: Some(1),
            embedding_model: Some("EmbeddingGemma-300M".to_string()),
            embedding_attempts: 1,
            created_at: None,
            processing_started_at: None,
            indexed_at: None,
            indexing_provenance: Some(provenance),
            error_message: None,
            retry_count: 0,
        }
    }

    fn assert_document_matches(actual: &Document, expected: &Document) {
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    async fn temporary_pool() -> (sqlx::SqlitePool, std::path::PathBuf) {
        let sequence = TEST_DATABASE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-persistence-{}-{sequence}",
            std::process::id()
        ));
        let pool = init_pool(&path)
            .await
            .expect("temporary database should migrate");
        (pool, path)
    }

    #[tokio::test]
    async fn document_queries_share_full_field_and_legacy_null_semantics() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let mut full = indexed_document("full", provenance.clone());
        full.file_size_bytes = Some(1234);
        full.file_hash = Some(vec![1, 2, 3]);
        full.chunk_count = Some(7);
        full.embedding_attempts = 2;
        full.created_at = Some("2026-01-03 00:00:00".to_string());
        full.processing_started_at = Some("2026-01-03 00:01:00".to_string());
        full.indexed_at = Some("2026-01-03 00:02:00".to_string());
        full.error_message = Some("retained diagnostic".to_string());
        full.retry_count = 3;

        let mut nullable = indexed_document("nullable", provenance.clone());
        nullable.status = DocumentStatus::Queued;
        nullable.file_size_bytes = None;
        nullable.file_hash = Some(vec![4, 5, 6]);
        nullable.chunk_count = None;
        nullable.embedding_model = None;
        nullable.embedding_attempts = 0;
        nullable.indexing_provenance = None;
        let mut no_hash = nullable.clone();
        no_hash.id = "no-hash".to_string();
        no_hash.filename = "no-hash.pdf".to_string();
        no_hash.storage_path = "no-hash.pdf".to_string();
        no_hash.file_hash = None;

        let mut partial = indexed_document("partial", provenance);
        partial.file_hash = Some(vec![7, 8, 9]);
        partial.created_at = Some("2026-01-02 00:00:00".to_string());
        for document in [&full, &nullable, &no_hash, &partial] {
            documents.insert(document).await.unwrap();
        }
        sqlx::query(
            "UPDATE documents SET embedding_attempts = NULL, retry_count = NULL
             WHERE id IN ('nullable', 'no-hash')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE documents SET indexed_chunk_overlap = NULL WHERE id = 'partial'")
            .execute(&pool)
            .await
            .unwrap();
        partial.indexing_provenance = None;

        let listed = documents.list_all().await.unwrap();
        assert_eq!(listed.len(), 4);
        assert_eq!(listed[0].id, "full");
        assert_eq!(listed[1].id, "partial");
        for expected in [&full, &nullable, &no_hash, &partial] {
            let by_id = documents.get_by_id(&expected.id).await.unwrap().unwrap();
            assert_document_matches(&by_id, expected);
            let from_list = listed.iter().find(|row| row.id == expected.id).unwrap();
            assert_document_matches(from_list, expected);
            if let Some(hash) = &expected.file_hash {
                let by_hash = documents.find_by_hash(hash).await.unwrap().unwrap();
                assert_document_matches(&by_hash, expected);
            }
        }
        assert!(documents.get_by_id("missing").await.unwrap().is_none());
        assert!(documents.find_by_hash(&[]).await.unwrap().is_none());

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn indexed_reset_paths_share_assignments_and_preserve_status_guards() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let original = IndexProvenance {
            extraction_version: "token-aware-v1".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let desired = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            ..original.clone()
        };
        let mut guards = Vec::new();
        for (id, status) in [
            ("queued", DocumentStatus::Queued),
            ("embedding", DocumentStatus::Embedding),
            ("deleting", DocumentStatus::Deleting),
            ("failed", DocumentStatus::Failed),
        ] {
            let mut guard = indexed_document(id, original.clone());
            guard.status = status;
            documents.insert(&guard).await.unwrap();
            guards.push(guard);
        }

        for reset_path in ["by-ids", "vector-loss", "stale-provenance"] {
            let mut expected = indexed_document(reset_path, original.clone());
            expected.file_hash = Some(reset_path.as_bytes().to_vec());
            expected.created_at = Some("2026-01-01 00:00:00".to_string());
            expected.processing_started_at = Some("2026-01-01 00:01:00".to_string());
            expected.indexed_at = Some("2026-01-01 00:02:00".to_string());
            expected.error_message = Some("old failure".to_string());
            expected.retry_count = 3;
            documents.insert(&expected).await.unwrap();

            match reset_path {
                "by-ids" => {
                    let mut ids: Vec<String> =
                        guards.iter().map(|guard| guard.id.clone()).collect();
                    ids.push(expected.id.clone());
                    ids.push("missing".to_string());
                    documents
                        .requeue_indexed_documents_by_ids(&ids)
                        .await
                        .unwrap();
                }
                "vector-loss" => assert_eq!(
                    documents
                        .requeue_indexed_documents_after_vector_loss()
                        .await
                        .unwrap(),
                    vec![expected.id.clone()]
                ),
                "stale-provenance" => assert_eq!(
                    documents
                        .requeue_indexed_with_stale_provenance(&desired)
                        .await
                        .unwrap(),
                    vec![expected.id.clone()]
                ),
                _ => unreachable!(),
            }

            expected.status = DocumentStatus::Queued;
            expected.chunk_count = None;
            expected.embedding_model = None;
            expected.embedding_attempts = 0;
            expected.processing_started_at = None;
            expected.indexed_at = None;
            expected.error_message = None;
            expected.retry_count = 0;
            assert_document_matches(
                &documents.get_by_id(&expected.id).await.unwrap().unwrap(),
                &expected,
            );
            for guard in &guards {
                assert_document_matches(
                    &documents.get_by_id(&guard.id).await.unwrap().unwrap(),
                    guard,
                );
            }
        }

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn stale_provenance_selection_matches_reset_for_each_missing_or_changed_field() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let desired = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let cases = [
            (
                "changed-version",
                Some("token-aware-v1"),
                Some(512),
                Some(80),
            ),
            ("changed-size", Some("token-aware-v2"), Some(640), Some(80)),
            (
                "changed-overlap",
                Some("token-aware-v2"),
                Some(512),
                Some(64),
            ),
            ("missing-version", None, Some(512), Some(80)),
            ("missing-size", Some("token-aware-v2"), None, Some(80)),
            ("missing-overlap", Some("token-aware-v2"), Some(512), None),
        ];
        for (id, version, size, overlap) in cases {
            documents
                .insert(&indexed_document(id, desired.clone()))
                .await
                .unwrap();
            sqlx::query(
                "UPDATE documents SET indexed_extraction_version = ?,
                 indexed_chunk_size = ?, indexed_chunk_overlap = ? WHERE id = ?",
            )
            .bind(version)
            .bind(size)
            .bind(overlap)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut ids = documents
            .requeue_indexed_with_stale_provenance(&desired)
            .await
            .unwrap();
        ids.sort();
        let mut expected_ids: Vec<String> = cases.iter().map(|case| case.0.to_string()).collect();
        expected_ids.sort();
        assert_eq!(ids, expected_ids);
        for (id, version, size, overlap) in cases {
            let document = documents.get_by_id(id).await.unwrap().unwrap();
            assert_eq!(document.status, DocumentStatus::Queued);
            assert_eq!(document.chunk_count, None);
            let retained = sqlx::query_as::<_, (Option<String>, Option<i32>, Option<i32>)>(
                "SELECT indexed_extraction_version, indexed_chunk_size, indexed_chunk_overlap
                 FROM documents WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(retained, (version.map(str::to_string), size, overlap));
        }

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn requeues_only_indexed_documents_with_stale_provenance() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let desired = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let stale = IndexProvenance {
            extraction_version: "token-aware-v1".to_string(),
            ..desired.clone()
        };

        documents
            .insert(&indexed_document("current", desired.clone()))
            .await
            .unwrap();
        let mut stale_document = indexed_document("stale", stale.clone());
        stale_document.retry_count = 2;
        stale_document.embedding_attempts = 3;
        documents.insert(&stale_document).await.unwrap();

        assert_eq!(
            documents
                .requeue_indexed_with_stale_provenance(&desired)
                .await
                .unwrap(),
            vec!["stale"]
        );

        let current = documents.get_by_id("current").await.unwrap().unwrap();
        assert_eq!(current.status, DocumentStatus::Indexed);
        assert_eq!(current.indexing_provenance, Some(desired));

        let requeued = documents.get_by_id("stale").await.unwrap().unwrap();
        assert_eq!(requeued.status, DocumentStatus::Queued);
        assert_eq!(requeued.chunk_count, None);
        assert_eq!(requeued.embedding_model, None);
        assert_eq!(requeued.indexing_provenance, Some(stale));
        assert_eq!(requeued.retry_count, 0);
        assert_eq!(requeued.embedding_attempts, 0);

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn caps_recovery_of_interrupted_embedding_attempts() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };

        let mut retryable = indexed_document("retryable", provenance.clone());
        retryable.status = DocumentStatus::Embedding;
        retryable.embedding_attempts = 2;
        retryable.processing_started_at = Some("2026-01-01T00:00:00Z".to_string());
        documents.insert(&retryable).await.unwrap();

        let mut exhausted = indexed_document("exhausted", provenance.clone());
        exhausted.status = DocumentStatus::Embedding;
        exhausted.embedding_attempts = 3;
        exhausted.processing_started_at = Some("2026-01-01T00:00:00Z".to_string());
        documents.insert(&exhausted).await.unwrap();

        let mut ordinary_failure = indexed_document("ordinary-failure", provenance);
        ordinary_failure.status = DocumentStatus::Failed;
        ordinary_failure.retry_count = 2;
        documents.insert(&ordinary_failure).await.unwrap();

        assert_eq!(
            documents.reset_interrupted_embeddings(3).await.unwrap(),
            vec!["retryable"]
        );

        let retryable = documents.get_by_id("retryable").await.unwrap().unwrap();
        assert_eq!(retryable.status, DocumentStatus::Queued);
        assert_eq!(retryable.embedding_attempts, 2);
        assert_eq!(retryable.processing_started_at, None);

        let exhausted = documents.get_by_id("exhausted").await.unwrap().unwrap();
        assert_eq!(exhausted.status, DocumentStatus::Failed);
        assert_eq!(exhausted.embedding_attempts, 3);
        assert_eq!(exhausted.retry_count, 3);
        assert_eq!(exhausted.processing_started_at, None);
        assert_eq!(
            exhausted.error_message.as_deref(),
            Some(
                "Document processing reached the automatic recovery limit. Remove it and upload again to retry."
            )
        );

        assert_eq!(
            documents.reset_failed_to_queued(3).await.unwrap(),
            vec!["ordinary-failure"]
        );
        assert_eq!(
            documents
                .get_by_id("ordinary-failure")
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Queued
        );
        assert_eq!(
            documents
                .get_by_id("exhausted")
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Failed
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn starts_embedding_from_a_legacy_null_attempt_count() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let document = indexed_document("legacy-null-attempts", provenance);
        documents.insert(&document).await.unwrap();
        sqlx::query("UPDATE documents SET embedding_attempts = NULL WHERE id = ?")
            .bind(&document.id)
            .execute(&pool)
            .await
            .unwrap();

        documents.mark_embedding(&document.id).await.unwrap();

        let document = documents.get_by_id(&document.id).await.unwrap().unwrap();
        assert_eq!(document.status, DocumentStatus::Embedding);
        assert_eq!(document.embedding_attempts, 1);

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn requeues_an_interruption_after_a_successful_indexing_run() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let prior = IndexProvenance {
            extraction_version: "token-aware-v1".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let desired = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            ..prior.clone()
        };
        let mut document = indexed_document("previous-success", prior.clone());
        document.status = DocumentStatus::Embedding;
        document.embedding_attempts = 3;
        documents.insert(&document).await.unwrap();

        documents
            .mark_indexed(&document.id, 1, "EmbeddingGemma-300M", &prior)
            .await
            .unwrap();
        assert_eq!(
            documents
                .get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .embedding_attempts,
            0
        );

        assert_eq!(
            documents
                .requeue_indexed_with_stale_provenance(&desired)
                .await
                .unwrap(),
            vec![document.id.clone()]
        );
        documents.mark_embedding(&document.id).await.unwrap();

        assert_eq!(
            documents.reset_interrupted_embeddings(3).await.unwrap(),
            vec![document.id.clone()]
        );
        let document = documents.get_by_id(&document.id).await.unwrap().unwrap();
        assert_eq!(document.status, DocumentStatus::Queued);
        assert_eq!(document.embedding_attempts, 1);

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn maps_file_hash_uniqueness_conflicts_to_duplicate_document() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let mut first = indexed_document("first", provenance.clone());
        first.file_hash = Some(vec![42; 64]);
        let mut concurrent = indexed_document("concurrent", provenance.clone());
        concurrent.file_hash = first.file_hash.clone();

        documents.insert(&first).await.unwrap();
        assert_eq!(
            documents.insert(&concurrent).await.unwrap_err().code,
            domain::ErrorCode::DuplicateDocument
        );
        let mut id_collision = indexed_document("first", provenance);
        id_collision.file_hash = Some(vec![43; 64]);
        assert_eq!(
            documents.insert(&id_collision).await.unwrap_err().code,
            domain::ErrorCode::Storage
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn external_inference_config_persists_base_url_and_model() {
        let (pool, path) = temporary_pool().await;
        let settings = AppSettingsRepository::new(pool.clone());

        settings
            .set_external_inference_config("https://inference.example/v1", "model-a")
            .await
            .unwrap();

        assert_eq!(
            settings.get("external_base_url").await.unwrap().as_deref(),
            Some("https://inference.example/v1")
        );
        assert_eq!(
            settings.get("external_model").await.unwrap().as_deref(),
            Some("model-a")
        );
        settings
            .set_external_inference_config("https://replacement.example/v1", "model-b")
            .await
            .unwrap();
        assert_eq!(
            settings.get("external_base_url").await.unwrap().as_deref(),
            Some("https://replacement.example/v1")
        );
        assert_eq!(
            settings.get("external_model").await.unwrap().as_deref(),
            Some("model-b")
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn external_inference_config_rolls_back_when_second_upsert_fails() {
        let (pool, path) = temporary_pool().await;
        let settings = AppSettingsRepository::new(pool.clone());
        settings
            .set("external_base_url", "original-url")
            .await
            .unwrap();
        settings
            .set("external_model", "original-model")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_external_model_update
             BEFORE UPDATE ON app_settings WHEN NEW.key = 'external_model'
             BEGIN SELECT RAISE(ABORT, 'test model update failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = settings
            .set_external_inference_config("replacement-url", "replacement-model")
            .await
            .unwrap_err();
        assert_eq!(error.code, domain::ErrorCode::Storage);
        assert_eq!(
            settings.get("external_base_url").await.unwrap().as_deref(),
            Some("original-url")
        );
        assert_eq!(
            settings.get("external_model").await.unwrap().as_deref(),
            Some("original-model")
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn indexing_setting_rolls_back_when_document_requeue_fails() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let settings = AppSettingsRepository::new(pool.clone());
        let original = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let desired = IndexProvenance {
            chunk_size: 640,
            ..original.clone()
        };
        let document = indexed_document("document", original);
        documents.insert(&document).await.unwrap();
        settings.set("chunk_size", "512").await.unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_document_requeue
             BEFORE UPDATE ON documents WHEN NEW.status = 'queued'
             BEGIN SELECT RAISE(ABORT, 'test requeue failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = settings
            .set_indexing_setting_and_requeue_stale_documents("chunk_size", "640", &desired)
            .await
            .unwrap_err();
        assert_eq!(error.code, domain::ErrorCode::Storage);
        assert_eq!(
            settings.get("chunk_size").await.unwrap().as_deref(),
            Some("512")
        );
        assert_document_matches(
            &documents.get_by_id(&document.id).await.unwrap().unwrap(),
            &document,
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn settings_delete_removes_entry() {
        let (pool, path) = temporary_pool().await;
        let settings = AppSettingsRepository::new(pool.clone());

        settings.set("test_key", "test_value").await.unwrap();
        assert_eq!(
            settings.get("test_key").await.unwrap().as_deref(),
            Some("test_value")
        );

        settings.delete("test_key").await.unwrap();
        assert_eq!(settings.get("test_key").await.unwrap(), None);

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn chunk_setting_update_persists_before_requeueing_stale_documents() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let settings = AppSettingsRepository::new(pool.clone());
        let original = IndexProvenance {
            extraction_version: "token-aware-v2".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let desired = IndexProvenance {
            chunk_size: 640,
            ..original.clone()
        };

        documents
            .insert(&indexed_document("document", original))
            .await
            .unwrap();

        assert_eq!(
            settings
                .set_indexing_setting_and_requeue_stale_documents("chunk_size", "640", &desired,)
                .await
                .unwrap(),
            vec!["document"]
        );
        assert_eq!(
            settings.get("chunk_size").await.unwrap().as_deref(),
            Some("640")
        );
        assert_eq!(
            documents
                .get_by_id("document")
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Queued
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn mark_indexed_does_not_overwrite_deleting_status() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v1".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let mut document = indexed_document("deleting-doc", provenance.clone());
        document.status = DocumentStatus::Deleting;
        documents.insert(&document).await.unwrap();

        let updated = documents
            .mark_indexed(&document.id, 1, "EmbeddingGemma-300M", &provenance)
            .await
            .unwrap();

        assert!(!updated);
        assert_eq!(
            documents
                .get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Deleting
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn mark_embedding_and_failed_do_not_overwrite_deleting_status() {
        let (pool, path) = temporary_pool().await;
        let documents = DocumentRepository::new(pool.clone());
        let provenance = IndexProvenance {
            extraction_version: "token-aware-v1".to_string(),
            chunk_size: 512,
            chunk_overlap: 80,
        };
        let mut document = indexed_document("deleting-doc-2", provenance.clone());
        document.status = DocumentStatus::Deleting;
        documents.insert(&document).await.unwrap();

        let updated_embedding = documents.mark_embedding(&document.id).await.unwrap();
        assert!(!updated_embedding);
        assert_eq!(
            documents
                .get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Deleting
        );

        documents
            .mark_failed(&document.id, "synthetic failure")
            .await
            .unwrap();
        assert_eq!(
            documents
                .get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Deleting
        );

        pool.close().await;
        std::fs::remove_dir_all(path).unwrap();
    }
}
