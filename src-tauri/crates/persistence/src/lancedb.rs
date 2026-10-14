use arrow_array::{Float32Array, Int32Array, StringArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use domain::AppError;
use futures::stream::StreamExt;
use lancedb::connection::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use log::warn;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const IVF_PQ_MINIMUM_VECTORS: usize = 256;

static TABLE_CREATION_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

static VECTOR_WRITE_LOCK: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(1));

fn escape_sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn delete_document_filter(document_id: &str) -> String {
    format!("document_id = '{}'", escape_sql_string_literal(document_id))
}

fn indexed_document_filter(document_ids: &[String]) -> Option<String> {
    (!document_ids.is_empty()).then(|| {
        let ids = document_ids
            .iter()
            .map(|id| format!("'{}'", escape_sql_string_literal(id)))
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

fn vector_err(operation: impl Into<String>, error: impl std::fmt::Display) -> AppError {
    AppError::vector_db(
        operation,
        error.to_string(),
        "Verify that vector database files in .lancedb are intact and not locked by another process.",
    )
}

impl VectorDb {
    pub async fn init(path: &std::path::Path) -> Result<Self, AppError> {
        let uri = path.to_str().ok_or_else(|| {
            AppError::internal(
                "Initialize vector database",
                "Invalid vector DB path: path is not valid UTF-8",
                "Ensure the application data directory path contains valid UTF-8 characters.",
            )
        })?;
        let conn = lancedb::connect(uri)
            .execute()
            .await
            .map_err(|e| vector_err("Connect to vector database", e))?;

        Ok(Self {
            conn,
            table_exists: AtomicBool::new(false),
            fts_index_built: AtomicBool::new(false),
            vector_index_built: AtomicBool::new(false),
        })
    }
    pub async fn chunks_table_exists(&self) -> Result<bool, AppError> {
        let table_names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| vector_err("Check chunks table existence", e))?;
        Ok(table_names.iter().any(|name| name == "chunks"))
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
            .map_err(|e| vector_err("List table names", e))?;

        let table = if table_names.iter().any(|name| name == "chunks") {
            self.open_table().await?
        } else {
            self.conn
                .create_empty_table("chunks", Self::schema())
                .execute()
                .await
                .map_err(|e| vector_err("Create chunks table", e))?
        };

        self.table_exists.store(true, Ordering::Release);

        Ok(table)
    }

    async fn open_table(&self) -> Result<lancedb::Table, AppError> {
        self.conn
            .open_table("chunks")
            .execute()
            .await
            .map_err(|e| vector_err("Open chunks table", e))
    }

    pub async fn delete_chunks(&self, document_id: &str) -> Result<(), AppError> {
        let _permit = VECTOR_WRITE_LOCK.acquire().await.map_err(|_| {
            AppError::internal(
                "Delete vector chunks",
                "Failed to acquire vector write lock",
                "Wait for active vector operations to complete and retry.",
            )
        })?;

        let table_names = self
            .conn
            .table_names()
            .execute()
            .await
            .map_err(|e| vector_err("List table names for deletion", e))?;

        if table_names.iter().any(|name| name == "chunks") {
            let table = self
                .conn
                .open_table("chunks")
                .execute()
                .await
                .map_err(|e| vector_err("Open chunks table for deletion", e))?;

            table
                .delete(&delete_document_filter(document_id))
                .await
                .map_err(|e| vector_err("Delete document chunks", e))?;
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

        let _permit = VECTOR_WRITE_LOCK.acquire().await.map_err(|_| {
            AppError::internal(
                "Insert vector chunks",
                "Failed to acquire vector write lock",
                "Wait for ongoing vector writes to finish and retry.",
            )
        })?;

        let schema = Self::schema();

        let mut merge_insert = table.merge_insert(&["document_id", "page", "chunk_index"]);
        merge_insert.when_matched_update_all(None);
        merge_insert.when_not_matched_insert_all();

        merge_insert
            .execute(Box::new(arrow_array::RecordBatchIterator::new(
                batch_vec, schema,
            )))
            .await
            .map_err(|e| vector_err("Upsert chunks into vector table", e))?;

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
                    .map_err(|e| vector_err("List vector table indexes", e))?,
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
                    .map_err(|e| vector_err("Create FTS index on chunks table", e))?;
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
            .map_err(|e| vector_err("Count vectors in chunks table", e))?;
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
            .map_err(|e| vector_err("Create vector index on chunks table", e))?;
        wait_for_index(table, &lancedb::index::IndexType::IvfPq, "vector").await?;
        self.vector_index_built.store(true, Ordering::SeqCst);

        Ok(())
    }

    pub async fn compact(&self) -> Result<(), AppError> {
        let _permit = VECTOR_WRITE_LOCK.acquire().await.map_err(|_| {
            AppError::internal(
                "Compact vector store",
                "Failed to acquire vector write lock",
                "Wait for active queries to complete and retry.",
            )
        })?;

        if !self.chunks_table_exists().await? {
            return Ok(());
        }

        let table = self.open_table().await?;
        table
            .optimize(lancedb::table::OptimizeAction::default())
            .await
            .map_err(|e| vector_err("Compact chunks table", e))?;
        Ok(())
    }

    pub async fn hybrid_search(
        &self,
        query_text: &str,
        query_vector: &[f32],
        indexed_document_ids: &[String],
        limit: usize,
    ) -> Result<HybridSearchResult, AppError> {
        self.search(Some(query_text), query_vector, indexed_document_ids, limit)
            .await
    }

    /// Dense-only baseline with the same registry eligibility filter and result mapping.
    pub async fn vector_search(
        &self,
        query_vector: &[f32],
        indexed_document_ids: &[String],
        limit: usize,
    ) -> Result<HybridSearchResult, AppError> {
        self.search(None, query_vector, indexed_document_ids, limit)
            .await
    }

    async fn search(
        &self,
        query_text: Option<&str>,
        query_vector: &[f32],
        indexed_document_ids: &[String],
        limit: usize,
    ) -> Result<HybridSearchResult, AppError> {
        let requested_mode = if query_text.is_some() {
            RetrievalMode::Hybrid
        } else {
            RetrievalMode::VectorOnly
        };
        let Some(document_filter) = indexed_document_filter(indexed_document_ids) else {
            return Ok(HybridSearchResult {
                results: Vec::new(),
                retrieval_mode: requested_mode,
            });
        };

        if !self.chunks_table_exists().await? {
            return Ok(HybridSearchResult {
                results: Vec::new(),
                retrieval_mode: requested_mode,
            });
        }

        let table = self.open_table().await?;

        let vector_query = table
            .query()
            .nearest_to(query_vector.to_vec())
            .map_err(|e| vector_err("Execute vector search query", e))?
            .only_if(&document_filter)
            .limit(limit);

        static FTS_FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);

        let primary = if let Some(query_text) = query_text {
            vector_query
                .clone()
                .full_text_search(lancedb::index::scalar::FullTextSearchQuery::new(
                    query_text.to_string(),
                ))
                .execute()
                .await
        } else {
            vector_query.clone().execute().await
        };
        let (mut stream, retrieval_mode) = match primary {
            Ok(stream) => (stream, requested_mode),
            Err(e) if query_text.is_some() => {
                if !FTS_FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
                    warn!(
                        "hybrid search unavailable (FTS index missing) — falling back to vector-only search"
                    );
                }
                warn!("hybrid search error detail: {e}");
                let stream = vector_query
                    .execute()
                    .await
                    .map_err(|e| vector_err("Fetch vector search stream", e))?;
                (stream, RetrievalMode::VectorOnly)
            }
            Err(error) => return Err(vector_err("Execute vector search query", error)),
        };

        let mut out = Vec::new();

        while let Some(batch) = stream.next().await {
            let batch = batch.map_err(|e| vector_err("Read search record batch", e))?;

            let docs_array = batch
                .column_by_name("document_id")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| {
                    AppError::internal(
                        "Process search results",
                        "Missing document_id column in chunks batch",
                        "Re-index documents to regenerate the chunks table.",
                    )
                })?;
            let content_array = batch
                .column_by_name("content")
                .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| {
                    AppError::internal(
                        "Process search results",
                        "Missing content column in chunks batch",
                        "Re-index documents to regenerate the chunks table.",
                    )
                })?;
            let page_array = batch
                .column_by_name("page")
                .and_then(|a| a.as_any().downcast_ref::<Int32Array>())
                .ok_or_else(|| {
                    AppError::internal(
                        "Process search results",
                        "Missing page column in chunks batch",
                        "Re-index documents to regenerate the chunks table.",
                    )
                })?;
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

    /// Returns the number of stored chunks for each document without creating the table.
    pub async fn indexed_chunk_counts(&self) -> Result<HashMap<String, usize>, AppError> {
        if !self.chunks_table_exists().await? {
            return Ok(HashMap::new());
        }

        let table = self.open_table().await?;
        let mut stream = table
            .query()
            .select(lancedb::query::Select::columns(&["document_id"]))
            .execute()
            .await
            .map_err(|e| vector_err("Select document_id from chunks", e))?;
        let mut counts = HashMap::new();

        while let Some(batch) = stream.next().await {
            let batch = batch.map_err(|e| vector_err("Read chunk count batch", e))?;
            let document_ids = batch
                .column_by_name("document_id")
                .and_then(|array| array.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| {
                    AppError::internal(
                        "Count indexed chunks",
                        "Missing document_id column in chunks batch",
                        "Re-index documents to regenerate the chunks table.",
                    )
                })?;

            for index in 0..batch.num_rows() {
                *counts
                    .entry(document_ids.value(index).to_string())
                    .or_insert(0) += 1;
            }
        }

        Ok(counts)
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
        .map_err(|e| vector_err("List created indexes", e))?;
    let index_names: Vec<_> = indexes
        .iter()
        .filter(|index| {
            &index.index_type == index_type && index.columns.iter().any(|indexed| indexed == column)
        })
        .map(|index| index.name.as_str())
        .collect();

    if index_names.is_empty() {
        return Err(AppError::vector_db(
            "Verify vector index readiness",
            format!("Created {index_type} index for {column} was not found in LanceDB"),
            "Re-run index creation after documents have been indexed.",
        ));
    }

    table
        .wait_for_index(&index_names, std::time::Duration::from_secs(2 * 60 * 60))
        .await
        .map_err(|e| vector_err("Wait for vector index creation", e))
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
    use super::{VectorDb, delete_document_filter, indexed_document_filter, should_build_ivf_pq};
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

    #[test]
    fn escapes_document_id_delete_filter() {
        assert_eq!(
            delete_document_filter("quote'identifier"),
            "document_id = 'quote''identifier'"
        );
    }

    #[tokio::test]
    async fn checks_for_chunks_table_without_creating_it() {
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-chunks-table-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = VectorDb::init(&path).await.unwrap();

        assert!(!db.chunks_table_exists().await.unwrap());
        db.get_or_create_table().await.unwrap();
        assert!(db.chunks_table_exists().await.unwrap());

        drop(db);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn missing_table_reads_and_compaction_do_not_create_chunks_table() {
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-missing-chunks-table-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = VectorDb::init(&path).await.unwrap();

        assert!(db.indexed_chunk_counts().await.unwrap().is_empty());
        assert!(
            db.hybrid_search("content", &[0.0; 768], &["indexed".to_string()], 1)
                .await
                .unwrap()
                .results
                .is_empty()
        );
        db.compact().await.unwrap();
        assert!(!db.chunks_table_exists().await.unwrap());

        drop(db);
        std::fs::remove_dir_all(path).unwrap();
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
    async fn indexed_chunk_counts_group_rows_by_document_id() {
        let path = std::env::temp_dir().join(format!(
            "mrk-ragtag-indexed-chunk-counts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let db = VectorDb::init(&path).await.unwrap();
        db.upsert_chunks(vec![vector_batch(&["first", "first", "second"])])
            .await
            .unwrap();

        let counts = db.indexed_chunk_counts().await.unwrap();
        assert_eq!(counts.len(), 2);
        assert_eq!(counts.get("first"), Some(&2));
        assert_eq!(counts.get("second"), Some(&1));

        drop(db);
        std::fs::remove_dir_all(path).unwrap();
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

        let dense = db
            .vector_search(&vec![0.0; 768], &["indexed".to_string()], 2)
            .await
            .unwrap();
        assert_eq!(dense.retrieval_mode, super::RetrievalMode::VectorOnly);
        assert_eq!(dense.results.len(), 1);
        assert_eq!(dense.results[0].document_id, "indexed");
        let empty = db.vector_search(&vec![0.0; 768], &[], 2).await.unwrap();
        assert!(empty.results.is_empty());
        assert_eq!(empty.retrieval_mode, super::RetrievalMode::VectorOnly);

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
