use arrow_array::{Float32Array, Int32Array, StringArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use domain::AppError;
use futures::stream::StreamExt;
use lancedb::connection::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use log::warn;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const IVF_PQ_MINIMUM_VECTORS: usize = 256;

static TABLE_CREATION_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

static VECTOR_WRITE_LOCK: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(1));

fn indexed_document_filter(document_ids: &[String]) -> Option<String> {
    (!document_ids.is_empty()).then(|| {
        let ids = document_ids
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!("document_id IN ({ids})")
    })
}

pub struct VectorDb {
    pub conn: Connection,
    table_exists: AtomicBool,
    fts_index_built: AtomicBool,
    vector_index_built: AtomicBool,
}

impl VectorDb {
    pub async fn init(path: &std::path::Path) -> Result<Self, AppError> {
        let uri = path
            .to_str()
            .ok_or_else(|| AppError::Internal("Invalid vector DB path".into()))?;
        let conn = lancedb::connect(uri)
            .execute()
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

        Ok(Self {
            conn,
            table_exists: AtomicBool::new(false),
            fts_index_built: AtomicBool::new(false),
            vector_index_built: AtomicBool::new(false),
        })
    }

    pub fn schema() -> Arc<ArrowSchema> {
        Arc::new(ArrowSchema::new(vec![
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 768),
                false,
            ),
            Field::new("document_id", DataType::Utf8, false),
            Field::new("page", DataType::Int32, false),
            Field::new("chunk_index", DataType::Int32, false),
            Field::new("content", DataType::Utf8, false),
        ]))
    }

    pub async fn get_or_create_table(&self) -> Result<lancedb::Table, AppError> {
        if self.table_exists.load(Ordering::Acquire) {
            return self.open_table().await;
        }

        let _guard = TABLE_CREATION_LOCK.lock().await;

        if self.table_exists.load(Ordering::Acquire) {
            return self.open_table().await;
        }

        let table_names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

        let table = if table_names.iter().any(|name| name == "chunks") {
            self.open_table().await?
        } else {
            self.conn
                .create_empty_table("chunks", Self::schema())
                .execute()
                .await
                .map_err(|e| AppError::VectorDb(e.to_string()))?
        };

        self.table_exists.store(true, Ordering::Release);

        Ok(table)
    }

    async fn open_table(&self) -> Result<lancedb::Table, AppError> {
        self.conn
            .open_table("chunks")
            .execute()
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))
    }

    pub async fn delete_chunks(&self, document_id: &str) -> Result<(), AppError> {
        let _permit = VECTOR_WRITE_LOCK
            .acquire()
            .await
            .map_err(|_| AppError::Internal("Failed to acquire vector write lock".into()))?;

        let table_names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

        if table_names.contains(&"chunks".to_string()) {
            let table = self
                .conn
                .open_table("chunks")
                .execute()
                .await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;

            table
                .delete(format!("document_id = '{}'", document_id).as_str())
                .await
                .map_err(|e| AppError::VectorDb(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn upsert_chunks(
        &self,
        batches: impl IntoIterator<Item = arrow_array::RecordBatch>,
    ) -> Result<(), AppError> {
        let table = self.get_or_create_table().await?;

        let batch_vec: Vec<_> = batches.into_iter().map(Ok).collect();
        if batch_vec.is_empty() {
            return Ok(());
        }

        let _permit = VECTOR_WRITE_LOCK
            .acquire()
            .await
            .map_err(|_| AppError::Internal("Failed to acquire vector write lock".into()))?;

        let schema = Self::schema();

        let mut merge_insert = table.merge_insert(&["document_id", "page", "chunk_index"]);
        merge_insert.when_matched_update_all(None);
        merge_insert.when_not_matched_insert_all();

        merge_insert
            .execute(Box::new(arrow_array::RecordBatchIterator::new(
                batch_vec, schema,
            )))
            .await
            .map_err(|e| AppError::VectorDb(e.to_string()))?;

        self.ensure_indexes(&table).await?;

        Ok(())
    }

    async fn ensure_indexes(&self, table: &lancedb::Table) -> Result<(), AppError> {
        let fts_index_built = self.fts_index_built.load(Ordering::SeqCst);
        let vector_index_built = self.vector_index_built.load(Ordering::SeqCst);

        let existing_indexes = if fts_index_built && vector_index_built {
            None
        } else {
            Some(
                table
                    .list_indices()
                    .await
                    .map_err(|e| AppError::VectorDb(format!("Failed to list indexes: {e}")))?,
            )
        };

        if !fts_index_built {
            if existing_indexes.as_ref().is_some_and(|indexes| {
                has_index(indexes, &lancedb::index::IndexType::FTS, "content")
            }) {
                self.fts_index_built.store(true, Ordering::SeqCst);
            } else {
                let fts_params =
                    lancedb::index::Index::FTS(lancedb::index::scalar::FtsIndexBuilder::default());
                table
                    .create_index(&["content"], fts_params)
                    .execute()
                    .await
                    .map_err(|e| AppError::VectorDb(format!("FTS index creation failed: {e}")))?;
                wait_for_index(table, &lancedb::index::IndexType::FTS, "content").await?;
                self.fts_index_built.store(true, Ordering::SeqCst);
            }
        }

        if vector_index_built {
            return Ok(());
        }

        if existing_indexes
            .as_ref()
            .is_some_and(|indexes| has_index(indexes, &lancedb::index::IndexType::IvfPq, "vector"))
        {
            self.vector_index_built.store(true, Ordering::SeqCst);
            return Ok(());
        }

        let vector_count = table
            .count_rows(None)
            .await
            .map_err(|e| AppError::VectorDb(format!("Failed to count vectors: {e}")))?;
        if !should_build_ivf_pq(vector_count) {
            return Ok(());
        }

        table
            .create_index(
                &["vector"],
                lancedb::index::Index::IvfPq(lancedb::index::vector::IvfPqIndexBuilder::default()),
            )
            .execute()
            .await
            .map_err(|e| AppError::VectorDb(format!("Vector index creation failed: {e}")))?;
        wait_for_index(table, &lancedb::index::IndexType::IvfPq, "vector").await?;
        self.vector_index_built.store(true, Ordering::SeqCst);

        Ok(())
    }

    pub async fn compact(&self) -> Result<(), AppError> {
        let table = self.get_or_create_table().await?;
        table
            .optimize(lancedb::table::OptimizeAction::default())
            .await
            .map_err(|e| AppError::VectorDb(format!("Compact failed: {e}")))?;
        Ok(())
    }

    pub async fn hybrid_search(
        &self,
        query_text: &str,
        query_vector: &[f32],
        indexed_document_ids: &[String],
        limit: usize,
    ) -> Result<HybridSearchResult, AppError> {
        let Some(document_filter) = indexed_document_filter(indexed_document_ids) else {
            return Ok(HybridSearchResult {
                results: Vec::new(),
                retrieval_mode: RetrievalMode::Hybrid,
            });
        };

        let table = self.get_or_create_table().await?;

        let query_vector_f32: Vec<f32> = query_vector.to_vec();
        let fts_query = lancedb::index::scalar::FullTextSearchQuery::new(query_text.to_string());

        static FTS_FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);

        let (mut stream, retrieval_mode) = match table
            .query()
            .full_text_search(fts_query)
            .nearest_to(query_vector_f32.clone())
            .map_err(|e| AppError::VectorDb(e.to_string()))?
            .only_if(&document_filter)
            .limit(limit)
            .execute()
            .await
        {
            Ok(stream) => (stream, RetrievalMode::Hybrid),
            Err(e) => {
                if !FTS_FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
                    warn!(
                        "hybrid search unavailable (FTS index missing) — falling back to vector-only search"
                    );
                }
                warn!("hybrid search error detail: {e}");
                let stream = table
                    .query()
                    .nearest_to(query_vector_f32)
                    .map_err(|e| AppError::VectorDb(e.to_string()))?
                    .only_if(&document_filter)
                    .limit(limit)
                    .execute()
                    .await
                    .map_err(|e| AppError::VectorDb(e.to_string()))?;
                (stream, RetrievalMode::VectorOnly)
            }
        };

        let mut out = Vec::new();

        while let Some(batch) = stream.next().await {
            let batch = batch.map_err(|e| AppError::VectorDb(e.to_string()))?;

            let docs_array = batch
                .column_by_name("document_id")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| AppError::Internal("Missing document_id".into()))?;
            let content_array = batch
                .column_by_name("content")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| AppError::Internal("Missing content".into()))?;
            let page_array = batch
                .column_by_name("page")
                .and_then(|a| a.as_any().downcast_ref::<Int32Array>())
                .ok_or_else(|| AppError::Internal("Missing page".into()))?;
            let distance_array = batch
                .column_by_name("_distance")
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

        let mut seen: HashSet<(String, u32, String)> = HashSet::new();
        out.retain(|r| seen.insert((r.document_id.clone(), r.page, r.content.clone())));
        out.truncate(limit);

        Ok(HybridSearchResult {
            results: out,
            retrieval_mode,
        })
    }
}

fn should_build_ivf_pq(vector_count: usize) -> bool {
    vector_count >= IVF_PQ_MINIMUM_VECTORS
}

fn has_index(
    indexes: &[lancedb::index::IndexConfig],
    index_type: &lancedb::index::IndexType,
    column: &str,
) -> bool {
    indexes.iter().any(|index| {
        &index.index_type == index_type && index.columns.iter().any(|indexed| indexed == column)
    })
}

async fn wait_for_index(
    table: &lancedb::Table,
    index_type: &lancedb::index::IndexType,
    column: &str,
) -> Result<(), AppError> {
    let indexes = table
        .list_indices()
        .await
        .map_err(|e| AppError::VectorDb(format!("Failed to list created indexes: {e}")))?;
    let index_names: Vec<_> = indexes
        .iter()
        .filter(|index| {
            &index.index_type == index_type && index.columns.iter().any(|indexed| indexed == column)
        })
        .map(|index| index.name.as_str())
        .collect();

    if index_names.is_empty() {
        return Err(AppError::VectorDb(format!(
            "Created {index_type} index for {column} was not found in LanceDB"
        )));
    }

    table
        .wait_for_index(&index_names, std::time::Duration::from_secs(2 * 60 * 60))
        .await
        .map_err(|e| AppError::VectorDb(format!("Index readiness check failed: {e}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalMode {
    /// LanceDB ranked this result with reciprocal-rank fusion, not a calibrated confidence.
    Hybrid,
    VectorOnly,
}

pub struct HybridSearchResult {
    pub results: Vec<SearchResult>,
    pub retrieval_mode: RetrievalMode,
}

pub struct SearchResult {
    pub document_id: String,
    pub content: String,
    pub page: u32,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::{VectorDb, indexed_document_filter, should_build_ivf_pq};
    use arrow_array::{FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray};
    use std::sync::Arc;

    #[test]
    fn ivf_pq_requires_at_least_256_vectors() {
        assert!(!should_build_ivf_pq(0));
        assert!(!should_build_ivf_pq(255));
        assert!(should_build_ivf_pq(256));
        assert!(should_build_ivf_pq(257));
    }

    #[test]
    fn filters_to_escaped_indexed_document_ids() {
        assert_eq!(indexed_document_filter(&[]), None);
        assert_eq!(
            indexed_document_filter(&["current".to_string(), "quote'identifier".to_string()]),
            Some("document_id IN ('current', 'quote''identifier')".to_string())
        );
    }

    fn vector_batch(document_ids: &[&str]) -> RecordBatch {
        let schema = VectorDb::schema();
        let arrow_schema::DataType::FixedSizeList(inner_field, _) = schema.field(0).data_type()
        else {
            panic!("vector schema must use a fixed-size list");
        };
        let vectors = FixedSizeListArray::try_new(
            inner_field.clone(),
            768,
            Arc::new(Float32Array::from(vec![0.0; document_ids.len() * 768])),
            None,
        )
        .unwrap();
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(vectors),
                Arc::new(StringArray::from(document_ids.to_vec())),
                Arc::new(Int32Array::from(vec![1; document_ids.len()])),
                Arc::new(Int32Array::from(
                    (0..document_ids.len() as i32).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    document_ids
                        .iter()
                        .map(|id| format!("content for {id}"))
                        .collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn hybrid_search_filters_lifecycle_before_limiting_candidates() {
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-lifecycle-filter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = VectorDb::init(&path).await.unwrap();
        db.upsert_chunks(vec![vector_batch(&["indexed", "stale"])])
            .await
            .unwrap();

        let result = db
            .hybrid_search("content", &vec![0.0; 768], &["indexed".to_string()], 2)
            .await
            .unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].document_id, "indexed");

        drop(db);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn concurrent_first_access_creates_the_chunks_table_once() {
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-lancedb-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time must be after the Unix epoch")
                .as_nanos()
        ));
        let db = Arc::new(
            VectorDb::init(&path)
                .await
                .expect("database must initialize"),
        );

        let results = futures::future::join_all((0..8).map(|_| {
            let db = Arc::clone(&db);
            async move { db.get_or_create_table().await }
        }))
        .await;

        assert!(results.into_iter().all(|result| result.is_ok()));

        let table_names = db
            .conn
            .table_names()
            .execute()
            .await
            .expect("table names must be readable");
        assert_eq!(
            table_names
                .iter()
                .filter(|name| name.as_str() == "chunks")
                .count(),
            1
        );

        drop(db);
        std::fs::remove_dir_all(path).expect("temporary database must be removable");
    }
}
