use domain::AppError;
use lancedb::connection::Connection;
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct VectorDb {
    pub conn: Connection,
    fts_built: AtomicBool,
}

impl VectorDb {
    pub async fn init(path: &std::path::Path) -> Result<Self, AppError> {
        let uri = path.to_str().ok_or_else(|| AppError::Internal("Invalid vector DB path".into()))?;
        let conn = lancedb::connect(uri).execute().await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;
        
        Ok(Self { conn, fts_built: AtomicBool::new(false) })
    }

    pub fn schema() -> Arc<ArrowSchema> {
        Arc::new(ArrowSchema::new(vec![
            Field::new("vector", DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                768
            ), false),
            Field::new("document_id", DataType::Utf8, false),
            Field::new("page", DataType::Int32, false),
            Field::new("chunk_index", DataType::Int32, false),
            Field::new("content", DataType::Utf8, false),
        ]))
    }

    pub async fn get_or_create_table(&self) -> Result<lancedb::Table, AppError> {
        let table_names = self.conn.table_names().execute().await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;
        
        let schema = Self::schema();
        
        if table_names.contains(&"chunks".to_string()) {
            let table = self.conn.open_table("chunks").execute().await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;
            Ok(table)
        } else {
            let table = self.conn.create_empty_table("chunks", schema).execute().await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;
            
            // FTS Index Creation MUST be created AFTER rows are inserted, but LanceDB 
            // also supports creating an empty index if we use wait_for_index().
            // However, per the architecture docs: empty table creation leads to a silently broken index.
            // We will defer FTS creation until after the first insert in the pipeline.
            Ok(table)
        }
    }

    pub async fn upsert_chunks(&self, batches: impl IntoIterator<Item = arrow_array::RecordBatch>) -> Result<(), AppError> {
        let table = self.get_or_create_table().await?;
        
        let batch_vec: Vec<_> = batches.into_iter().map(Ok).collect();
        if batch_vec.is_empty() {
            return Ok(());
        }

        // Use a static semaphore as required by the AGENTS.md document: 
        // "Use a tokio::sync::Semaphore(1) for all writes to prevent Commit conflict for version N"
        use tokio::sync::Semaphore;
        use std::sync::OnceLock;
        
        static WRITE_LOCK: OnceLock<Semaphore> = OnceLock::new();
        let semaphore = WRITE_LOCK.get_or_init(|| Semaphore::new(1));
        
        let _permit = semaphore.acquire().await.map_err(|_| AppError::Internal("Failed to acquire write lock".into()))?;

        let schema = Self::schema();

        // Merge insert ensures idempotency
        let mut merge_insert = table.merge_insert(&["document_id", "page", "chunk_index"]);
        merge_insert.when_matched_update_all(None);
        merge_insert.when_not_matched_insert_all();
        
        merge_insert.execute(Box::new(arrow_array::RecordBatchIterator::new(batch_vec, schema)))
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

        // Rebuild FTS index on first insert per session to ensure it's healthy
        if !self.fts_built.swap(true, Ordering::SeqCst) {
            let index_params = lancedb::index::Index::FTS(lancedb::index::scalar::FtsIndexBuilder::default());
            table.create_index(&["content"], index_params)
                .replace(true)
                .execute()
                .await
                .map_err(|e| AppError::VectorDb(format!("FTS Index failed: {}", e)))?;
        }

        Ok(())
    }
}
