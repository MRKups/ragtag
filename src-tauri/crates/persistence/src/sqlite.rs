use domain::{AppError, Document, DocumentStatus};
use sqlx::{sqlite::{SqliteConnectOptions, SqlitePoolOptions}, SqlitePool};
use std::path::Path;
use std::str::FromStr;

fn sql_err(e: sqlx::Error) -> AppError {
    AppError::Database(e.to_string())
}

pub async fn init_pool(app_local_data_dir: &Path) -> Result<SqlitePool, AppError> {
    std::fs::create_dir_all(app_local_data_dir)?;

    let db_path = app_local_data_dir.join("app.db");
    
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
        .map_err(sql_err)?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(sql_err)?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| AppError::Database(format!("Migration failed: {}", e)))?;

    Ok(pool)
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
            DocumentStatus::Failed => "failed",
        };

        sqlx::query!(
            r#"
            INSERT INTO documents (
                id, filename, file_type, file_size_bytes, file_hash, storage_path, 
                status, chunk_count, embedding_model, embedding_attempts, 
                created_at, processing_started_at, indexed_at, error_message, retry_count
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            doc.id,
            doc.filename,
            doc.file_type,
            doc.file_size_bytes,
            doc.file_hash,
            doc.storage_path,
            status_str,
            doc.chunk_count,
            doc.embedding_model,
            doc.embedding_attempts,
            doc.created_at,
            doc.processing_started_at,
            doc.indexed_at,
            doc.error_message,
            doc.retry_count
        )
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn update_status(&self, id: &str, status: DocumentStatus) -> Result<(), AppError> {
        let status_str = match status {
            DocumentStatus::Queued => "queued",
            DocumentStatus::Embedding => "embedding",
            DocumentStatus::Indexed => "indexed",
            DocumentStatus::Failed => "failed",
        };

        sqlx::query!(
            "UPDATE documents SET status = ? WHERE id = ?",
            status_str,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn mark_embedding(&self, id: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE documents
             SET status = 'embedding',
                 processing_started_at = CURRENT_TIMESTAMP,
                 embedding_attempts = embedding_attempts + 1,
                 error_message = NULL
             WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn mark_indexed(
        &self,
        id: &str,
        chunk_count: usize,
        embedding_model: &str,
    ) -> Result<(), AppError> {
        let chunk_count = i32::try_from(chunk_count)
            .map_err(|_| AppError::Database("Chunk count exceeds SQLite integer range".into()))?;

        sqlx::query(
            "UPDATE documents
             SET status = 'indexed',
                 chunk_count = ?,
                 embedding_model = ?,
                 indexed_at = CURRENT_TIMESTAMP,
                 error_message = NULL
             WHERE id = ?",
        )
        .bind(chunk_count)
        .bind(embedding_model)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<Document>, AppError> {
        let row = sqlx::query_as!(
            DocumentRow,
            r#"SELECT 
                id as "id!", 
                filename as "filename!", 
                file_type as "file_type!", 
                file_size_bytes, 
                file_hash, 
                storage_path as "storage_path!", 
                status as "status!", 
                chunk_count, 
                embedding_model, 
                embedding_attempts, 
                CAST(created_at as TEXT) as "created_at?: String", 
                CAST(processing_started_at as TEXT) as "processing_started_at?: String", 
                CAST(indexed_at as TEXT) as "indexed_at?: String", 
                error_message, 
                retry_count 
            FROM documents 
            WHERE id = ?"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(row.map(|r| r.into_domain()))
    }

    pub async fn get_queued_ids(&self) -> Result<Vec<String>, AppError> {
        let rows = sqlx::query!(
            "SELECT id FROM documents WHERE status = 'queued'"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(rows.into_iter().filter_map(|r| r.id).collect())
    }

    pub async fn list_all(&self) -> Result<Vec<Document>, AppError> {
        let rows = sqlx::query_as!(
            DocumentRow,
            r#"SELECT 
                id as "id!", 
                filename as "filename!", 
                file_type as "file_type!", 
                file_size_bytes, 
                file_hash, 
                storage_path as "storage_path!", 
                status as "status!", 
                chunk_count, 
                embedding_model, 
                embedding_attempts, 
                CAST(created_at as TEXT) as "created_at?: String", 
                CAST(processing_started_at as TEXT) as "processing_started_at?: String", 
                CAST(indexed_at as TEXT) as "indexed_at?: String", 
                error_message, 
                retry_count 
            FROM documents 
            ORDER BY created_at DESC"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(rows.into_iter().map(|r| r.into_domain()).collect())
    }

    pub async fn mark_failed(&self, id: &str, error_message: &str) -> Result<(), AppError> {
        sqlx::query!(
            "UPDATE documents SET status = 'failed', retry_count = retry_count + 1, error_message = ? WHERE id = ?",
            error_message,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(())
    }

    pub async fn find_by_hash(&self, hash: &[u8]) -> Result<Option<Document>, AppError> {
        let row = sqlx::query_as!(
            DocumentRow,
            r#"SELECT 
                id as "id!", 
                filename as "filename!", 
                file_type as "file_type!", 
                file_size_bytes, 
                file_hash, 
                storage_path as "storage_path!", 
                status as "status!", 
                chunk_count, 
                embedding_model, 
                embedding_attempts, 
                CAST(created_at as TEXT) as "created_at?: String", 
                CAST(processing_started_at as TEXT) as "processing_started_at?: String", 
                CAST(indexed_at as TEXT) as "indexed_at?: String", 
                error_message, 
                retry_count 
            FROM documents 
            WHERE file_hash = ?"#,
            hash
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(row.map(|r| r.into_domain()))
    }

    pub async fn delete_by_id(&self, id: &str) -> Result<(), AppError> {
        sqlx::query!("DELETE FROM documents WHERE id = ?", id)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        Ok(())
    }

    pub async fn reset_embedding_to_queued(&self) -> Result<Vec<String>, AppError> {
        let docs = sqlx::query!(
            "SELECT id FROM documents WHERE status = 'embedding'"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;

        let ids: Vec<String> = docs.into_iter().filter_map(|d| d.id).collect();

        if !ids.is_empty() {
            sqlx::query!(
                "UPDATE documents SET status = 'queued' WHERE status = 'embedding'"
            )
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        }

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
}

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
    error_message: Option<String>,
    retry_count: Option<i64>,
}

impl DocumentRow {
    fn into_domain(self) -> Document {
        let status = match self.status.as_str() {
            "embedding" => DocumentStatus::Embedding,
            "indexed" => DocumentStatus::Indexed,
            "failed" => DocumentStatus::Failed,
            _ => DocumentStatus::Queued,
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
        let row = sqlx::query!(
            r#"SELECT value FROM app_settings WHERE key = ?"#,
            key
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;

        Ok(row.map(|r| r.value))
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
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
}
