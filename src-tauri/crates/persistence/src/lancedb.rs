use domain::AppError;
use lancedb::connection::Connection;
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use arrow_array::{Float32Array, StringArray, Int32Array};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use lancedb::query::{ExecutableQuery, QueryBase};
use futures::stream::StreamExt;

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
            
            Ok(table)
        }
    }

    pub async fn delete_chunks(&self, document_id: &str) -> Result<(), AppError> {
        let table_names = self.conn.table_names().execute().await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;
        
        if table_names.contains(&"chunks".to_string()) {
            let table = self.conn.open_table("chunks").execute().await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;
            
            table.delete(format!("document_id = '{}'", document_id).as_str())
                .await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;
        }
        
        Ok(())
    }

    pub async fn upsert_chunks(&self, batches: impl IntoIterator<Item = arrow_array::RecordBatch>) -> Result<(), AppError> {
        let table = self.get_or_create_table().await?;
        
        let batch_vec: Vec<_> = batches.into_iter().map(Ok).collect();
        if batch_vec.is_empty() {
            return Ok(());
        }

        use tokio::sync::Semaphore;
        use std::sync::OnceLock;
        
        static WRITE_LOCK: OnceLock<Semaphore> = OnceLock::new();
        let semaphore = WRITE_LOCK.get_or_init(|| Semaphore::new(1));
        
        let _permit = semaphore.acquire().await.map_err(|_| AppError::Internal("Failed to acquire write lock".into()))?;

        let schema = Self::schema();

        let mut merge_insert = table.merge_insert(&["document_id", "page", "chunk_index"]);
        merge_insert.when_matched_update_all(None);
        merge_insert.when_not_matched_insert_all();
        
        merge_insert.execute(Box::new(arrow_array::RecordBatchIterator::new(batch_vec, schema)))
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

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

    pub async fn compact(&self) -> Result<(), AppError> {
        let table = self.get_or_create_table().await?;
        table.optimize(lancedb::table::OptimizeAction::default()).await
            .map_err(|e| AppError::VectorDb(format!("Compact failed: {e}")))?;
        Ok(())
    }

    pub async fn hybrid_search(&self, query_text: &str, query_vector: &[f32], limit: usize) -> Result<Vec<SearchResult>, AppError> {
        let table = self.get_or_create_table().await?;
        
        let query_vector_f32: Vec<f32> = query_vector.to_vec();
        let fts_query = lance_index::scalar::FullTextSearchQuery::new(query_text.to_string());

        let mut stream = match table.query()
            .full_text_search(fts_query)
            .nearest_to(query_vector_f32.clone())
            .map_err(|e| AppError::VectorDb(e.to_string()))?
            .limit(limit)
            .execute()
            .await 
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[LanceDB] Hybrid search failed, falling back to vector-only: {}", e);
                table.query()
                    .nearest_to(query_vector_f32)
                    .map_err(|e| AppError::VectorDb(e.to_string()))?
                    .limit(limit)
                    .execute()
                    .await
                    .map_err(|e| AppError::VectorDb(e.to_string()))?
            }
        };

        let mut out = Vec::new();

        while let Some(batch) = stream.next().await {
            let batch = batch.map_err(|e| AppError::VectorDb(e.to_string()))?;
            
            let docs_array = batch.column_by_name("document_id")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| AppError::Internal("Missing document_id".into()))?;
            let content_array = batch.column_by_name("content")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| AppError::Internal("Missing content".into()))?;
            let page_array = batch.column_by_name("page")
                .and_then(|a| a.as_any().downcast_ref::<Int32Array>())
                .ok_or_else(|| AppError::Internal("Missing page".into()))?;
            let distance_array = batch.column_by_name("_distance")
                .and_then(|a| a.as_any().downcast_ref::<Float32Array>());

            for i in 0..batch.num_rows() {
                out.push(SearchResult {
                    document_id: docs_array.value(i).to_string(),
                    content: content_array.value(i).to_string(),
                    page: page_array.value(i) as u32,
                    score: distance_array.map(|a| a.value(i)).unwrap_or(0.0),
                });
            }
        }

        out.dedup_by(|a, b| a.document_id == b.document_id && a.page == b.page && a.content == b.content);
        out.truncate(limit);
        
        Ok(out)
    }
}

pub struct SearchResult {
    pub document_id: String,
    pub content: String,
    pub page: u32,
    pub score: f32,
}
