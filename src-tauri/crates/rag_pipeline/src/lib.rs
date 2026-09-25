pub mod chunker;
pub mod extractor;

use arrow_array::{FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray};
use domain::{AppError, IngestionProgress, models::IndexProvenance};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use persistence::{DocumentRepository, VectorDb};
use std::path::Path;
use std::sync::Arc;

pub use chunker::Chunk;

const EMBEDDING_MAX_TOKENS: usize = 2_048;
const EMBEDDING_BATCH_SIZE: usize = 32;
const EMBEDDING_DIMENSION: usize = 768;
const MAX_DOCUMENT_CHUNKS: usize = 8_192;
const PASSAGE_PREFIX: &str = "passage: ";
// Chunker caps overlap at half a chunk. Two times the 16 MiB extraction cap,
// plus one prefix per bounded chunk, leaves headroom beneath the 50 MiB upload
// envelope while bounding all model input constructed for one document.
const MAX_EMBEDDING_INPUT_BYTES: usize =
    extractor::EXTRACTED_TEXT_MAX_BYTES * 2 + MAX_DOCUMENT_CHUNKS * PASSAGE_PREFIX.len();

pub fn init_embedder(cache_dir: &Path) -> Result<TextEmbedding, AppError> {
    std::fs::create_dir_all(cache_dir)?;
    TextEmbedding::try_new(
        TextInitOptions::new(EmbeddingModel::EmbeddingGemma300M)
            .with_max_length(EMBEDDING_MAX_TOKENS)
            .with_cache_dir(cache_dir.to_path_buf()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to init embedder: {e}")))
}

pub struct DocumentIngestion<'a> {
    pub source_path: &'a Path,
    pub file_type: &'a str,
    pub document_id: &'a str,
    pub provenance: IndexProvenance,
}

pub async fn process_document(
    doc: DocumentIngestion<'_>,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    on_progress: Arc<dyn Fn(IngestionProgress) + Send + Sync>,
) -> Result<(), AppError> {
    let DocumentIngestion {
        source_path,
        file_type,
        document_id,
        provenance,
    } = doc;
    repo.mark_embedding(document_id).await?;

    let source_path = source_path.to_path_buf();
    let file_type = file_type.to_string();
    let extraction_progress = Arc::clone(&on_progress);
    let extraction_document_id = document_id.to_string();
    let extracted_pages = tokio::task::spawn_blocking(move || {
        extractor::extract_pages(
            &source_path,
            &file_type,
            move |current_page, total_pages| {
                extraction_progress(IngestionProgress::Extraction {
                    document_id: extraction_document_id.clone(),
                    current_page,
                    total_pages,
                });
            },
        )
    })
    .await
    .map_err(|e| AppError::Internal(format!("Extraction task panicked: {e}")))??;
    let (tokenizer, embedding_token_limit) = {
        let model = embedder
            .lock()
            .map_err(|e| AppError::Internal(format!("Embedder lock poisoned: {e}")))?;
        let embedding_token_limit = model
            .tokenizer
            .get_truncation()
            .map(|truncation| truncation.max_length)
            .ok_or_else(|| {
                AppError::Internal("Embedder tokenizer has no truncation limit".into())
            })?;
        (model.tokenizer.clone(), embedding_token_limit)
    };
    let chunk_size = usize::try_from(provenance.chunk_size)
        .map_err(|_| AppError::InvalidInput("chunk_size must be positive".into()))?;
    let chunk_overlap = usize::try_from(provenance.chunk_overlap)
        .map_err(|_| AppError::InvalidInput("chunk_overlap must be non-negative".into()))?;
    if chunk_overlap >= chunk_size {
        return Err(AppError::InvalidInput(
            "chunk_overlap must be smaller than chunk_size".into(),
        ));
    }

    let chunking_progress = Arc::clone(&on_progress);
    let chunking_document_id = document_id.to_string();
    let total_pages = extracted_pages.total_pages;
    let chunks = tokio::task::spawn_blocking(move || {
        let chunks = chunker::chunk_pages_with_progress(
            extracted_pages.pages,
            embedding_token_limit.min(chunk_size),
            chunk_overlap,
            tokenizer,
            move |current_page, chunks_on_page, total_chunks| {
                chunking_progress(IngestionProgress::Chunking {
                    document_id: chunking_document_id.clone(),
                    current_page,
                    total_pages,
                    chunks_on_page,
                    total_chunks,
                });
            },
        )?;
        if chunks.is_empty() {
            return Err(AppError::Internal("No text chunks produced".into()));
        }
        Ok(chunks)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Chunking task panicked: {e}")))??;
    ensure_chunk_budget(&chunks)?;

    let (chunks, embeddings) = {
        let embedder = Arc::clone(&embedder);
        let embedding_progress = Arc::clone(&on_progress);
        let embedding_document_id = document_id.to_string();
        tokio::task::spawn_blocking(move || {
            let total_chunks = chunks.len();
            let total_batches = embedding_batch_count(total_chunks);
            let mut embeddings = Vec::new();
            embeddings.try_reserve_exact(total_chunks).map_err(|e| {
                AppError::Internal(format!("Could not allocate embedding results: {e}"))
            })?;

            for (batch_index, chunk_batch) in chunks.chunks(EMBEDDING_BATCH_SIZE).enumerate() {
                let texts = chunk_batch
                    .iter()
                    .map(|chunk| format!("{PASSAGE_PREFIX}{}", chunk.content))
                    .collect::<Vec<_>>();
                let batch_embeddings = {
                    let mut model = embedder
                        .lock()
                        .map_err(|e| AppError::Internal(format!("Embedder lock poisoned: {e}")))?;
                    model
                        .embed(texts, Some(EMBEDDING_BATCH_SIZE))
                        .map_err(|e| AppError::Internal(format!("Embedding failed: {e}")))?
                };
                validate_embedding_shapes(chunk_batch, &batch_embeddings)?;
                embeddings.extend(batch_embeddings);
                embedding_progress(IngestionProgress::Embedding {
                    document_id: embedding_document_id.clone(),
                    current_batch: batch_index + 1,
                    total_batches,
                    embedded_chunks: embeddings.len(),
                    total_chunks,
                });
            }

            Ok::<_, AppError>((chunks, embeddings))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Embedding task panicked: {e}")))?
    }?;

    let batch = build_record_batch(document_id, &chunks, &embeddings)?;

    vector_db.delete_chunks(document_id).await?;
    vector_db.upsert_chunks(std::iter::once(batch)).await?;

    repo.mark_indexed(
        document_id,
        chunks.len(),
        "EmbeddingGemma-300M",
        &provenance,
    )
    .await?;
    on_progress(IngestionProgress::Stored {
        document_id: document_id.to_string(),
        chunk_count: chunks.len(),
    });

    Ok(())
}
fn embedding_batch_count(total_chunks: usize) -> usize {
    total_chunks.div_ceil(EMBEDDING_BATCH_SIZE)
}

fn ensure_chunk_budget(chunks: &[Chunk]) -> Result<(), AppError> {
    if chunks.len() > MAX_DOCUMENT_CHUNKS {
        return Err(AppError::InvalidInput(format!(
            "Document produces more than the {MAX_DOCUMENT_CHUNKS}-chunk indexing limit"
        )));
    }

    bounded_embedding_input_bytes(chunks.iter().map(|chunk| chunk.content.len()))?;
    Ok(())
}

fn bounded_embedding_input_bytes(
    content_lengths: impl IntoIterator<Item = usize>,
) -> Result<usize, AppError> {
    let total_input_bytes =
        content_lengths
            .into_iter()
            .try_fold(0usize, |total, content_len| {
                let input_bytes = content_len
                    .checked_add(PASSAGE_PREFIX.len())
                    .ok_or_else(embedding_input_limit_error)?;
                total
                    .checked_add(input_bytes)
                    .ok_or_else(embedding_input_limit_error)
            })?;
    if total_input_bytes > MAX_EMBEDDING_INPUT_BYTES {
        return Err(embedding_input_limit_error());
    }
    Ok(total_input_bytes)
}

fn embedding_input_limit_error() -> AppError {
    AppError::InvalidInput(format!(
        "Document embedding input exceeds the {MAX_EMBEDDING_INPUT_BYTES}-byte indexing limit"
    ))
}

fn embedding_shape_error(message: impl Into<String>) -> AppError {
    AppError::Internal(format!("Invalid embedding shape: {}", message.into()))
}

fn checked_embedding_value_count(num_chunks: usize) -> Result<usize, AppError> {
    num_chunks
        .checked_mul(EMBEDDING_DIMENSION)
        .ok_or_else(|| embedding_shape_error("embedding value count overflows usize"))
}

fn validate_embedding_shapes(chunks: &[Chunk], embeddings: &[Vec<f32>]) -> Result<usize, AppError> {
    if embeddings.len() != chunks.len() {
        return Err(embedding_shape_error(format!(
            "model returned {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        )));
    }
    if let Some((index, embedding)) = embeddings
        .iter()
        .enumerate()
        .find(|(_, embedding)| embedding.len() != EMBEDDING_DIMENSION)
    {
        return Err(embedding_shape_error(format!(
            "embedding {index} has dimension {}; expected {EMBEDDING_DIMENSION}",
            embedding.len()
        )));
    }
    checked_embedding_value_count(chunks.len())
}

type ReservedRecordBatchColumns = (Vec<f32>, Vec<String>, Vec<i32>, Vec<i32>, Vec<String>);

fn reserve_record_batch_columns(
    num_chunks: usize,
    embedding_values: usize,
) -> Result<ReservedRecordBatchColumns, AppError> {
    let mut all_floats = Vec::new();
    let mut doc_ids = Vec::new();
    let mut pages = Vec::new();
    let mut chunk_indices = Vec::new();
    let mut contents = Vec::new();
    all_floats
        .try_reserve_exact(embedding_values)
        .map_err(|e| embedding_shape_error(format!("could not allocate vector values: {e}")))?;
    doc_ids
        .try_reserve_exact(num_chunks)
        .map_err(|e| embedding_shape_error(format!("could not allocate document ids: {e}")))?;
    pages
        .try_reserve_exact(num_chunks)
        .map_err(|e| embedding_shape_error(format!("could not allocate page numbers: {e}")))?;
    chunk_indices
        .try_reserve_exact(num_chunks)
        .map_err(|e| embedding_shape_error(format!("could not allocate chunk indexes: {e}")))?;
    contents
        .try_reserve_exact(num_chunks)
        .map_err(|e| embedding_shape_error(format!("could not allocate chunk contents: {e}")))?;
    Ok((all_floats, doc_ids, pages, chunk_indices, contents))
}

fn build_record_batch(
    document_id: &str,
    chunks: &[Chunk],
    embeddings: &[Vec<f32>],
) -> Result<RecordBatch, AppError> {
    let num_chunks = chunks.len();
    let embedding_values = validate_embedding_shapes(chunks, embeddings)?;
    let schema = VectorDb::schema();
    let (mut all_floats, mut doc_ids, mut pages, mut chunk_indices, mut contents) =
        reserve_record_batch_columns(num_chunks, embedding_values)?;

    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        all_floats.extend_from_slice(embedding);
        doc_ids.push(document_id.to_string());
        pages.push(chunk.page);
        chunk_indices.push(chunk.chunk_index);
        contents.push(chunk.content.clone());
    }

    let float_values = Float32Array::from(all_floats);
    let arrow_schema::DataType::FixedSizeList(inner_field, _) = schema.field(0).data_type() else {
        return Err(AppError::Internal(
            "Schema vector field must be FixedSizeList".into(),
        ));
    };
    let vector_col = FixedSizeListArray::try_new(
        inner_field.clone(),
        EMBEDDING_DIMENSION as i32,
        Arc::new(float_values),
        None,
    )
    .map_err(|e| AppError::Internal(format!("Failed to build vector column: {e}")))?;

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(vector_col),
            Arc::new(StringArray::from(doc_ids)),
            Arc::new(Int32Array::from(pages)),
            Arc::new(Int32Array::from(chunk_indices)),
            Arc::new(StringArray::from(contents)),
        ],
    )
    .map_err(|e| AppError::Internal(format!("Failed to build RecordBatch: {e}")))
}

pub fn embed_query(
    embedder: &Arc<std::sync::Mutex<TextEmbedding>>,
    query: &str,
) -> Result<Vec<f32>, AppError> {
    let query_text = format!("query: {}", query);
    let mut model = embedder
        .lock()
        .map_err(|e| AppError::Internal(format!("Embedder lock poisoned: {e}")))?;
    let embeddings = model
        .embed(vec![query_text.as_str()], None)
        .map_err(|e| AppError::Internal(format!("Query embedding failed: {e}")))?;
    let embedding = embeddings
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal("No embedding returned".into()))?;
    if embedding.len() != EMBEDDING_DIMENSION {
        return Err(AppError::Internal(format!(
            "Query embedding dimension {} != {EMBEDDING_DIMENSION}",
            embedding.len()
        )));
    }
    Ok(embedding)
}

pub fn enhance_query(query: &str) -> String {
    const EXPANSIONS: &[(&str, &str)] = &[
        ("api", "application programming interface"),
        ("cpu", "central processing unit"),
        ("gpu", "graphics processing unit"),
        ("llm", "large language model"),
        ("sdk", "software development kit"),
        ("ui", "user interface"),
    ];

    let mut expansions = Vec::new();
    for token in query.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some((_, expansion)) = EXPANSIONS
            .iter()
            .find(|(abbreviation, _)| token.eq_ignore_ascii_case(abbreviation))
            .filter(|(_, exp)| !expansions.contains(exp))
        {
            expansions.push(*expansion);
        }
    }

    if expansions.is_empty() {
        query.to_string()
    } else {
        format!("{query} ({})", expansions.join(", "))
    }
}

pub fn build_rag_prompt(query: &str, chunks: &[domain::Citation]) -> String {
    let mut context = String::new();

    for (i, chunk) in chunks.iter().enumerate() {
        context.push_str(&format!(
            "[Source {}: {}, Page {}]\n{}\n\n",
            i + 1,
            chunk.filename,
            chunk.page,
            chunk.chunk_content
        ));
    }

    context.push_str(&format!("User question: {}", query));

    context
}

#[cfg(test)]
mod tests {
    use super::{
        EMBEDDING_DIMENSION, MAX_DOCUMENT_CHUNKS, MAX_EMBEDDING_INPUT_BYTES, PASSAGE_PREFIX,
        bounded_embedding_input_bytes, build_rag_prompt, checked_embedding_value_count,
        embedding_batch_count, enhance_query, ensure_chunk_budget, validate_embedding_shapes,
    };
    use crate::Chunk;

    #[test]
    fn numbers_retrieved_sources_in_rag_prompt() {
        let citations = vec![
            domain::Citation {
                document_id: "doc1".to_string(),
                filename: "manual.pdf".to_string(),
                storage_path: "manual.pdf".to_string(),
                page: 3,
                chunk_content: "First excerpt".to_string(),
                score: 0.9,
                is_hybrid: false,
            },
            domain::Citation {
                document_id: "doc2".to_string(),
                filename: "spec.docx".to_string(),
                storage_path: "spec.docx".to_string(),
                page: 1,
                chunk_content: "Second excerpt".to_string(),
                score: 0.8,
                is_hybrid: false,
            },
        ];
        let prompt = build_rag_prompt("How does it work?", &citations);
        assert!(prompt.contains("[Source 1: manual.pdf, Page 3]\nFirst excerpt\n\n"));
        assert!(prompt.contains("[Source 2: spec.docx, Page 1]\nSecond excerpt\n\n"));
        assert!(prompt.contains("User question: How does it work?"));
    }

    #[test]
    fn adds_unique_technical_expansions() {
        assert_eq!(
            enhance_query("GPU API gpu"),
            "GPU API gpu (graphics processing unit, application programming interface)"
        );
    }

    #[test]
    fn preserves_queries_without_known_abbreviations() {
        assert_eq!(enhance_query("installation guide"), "installation guide");
    }

    #[test]
    fn calculates_embedding_progress_batches_from_actual_chunk_totals() {
        assert_eq!(embedding_batch_count(1), 1);
        assert_eq!(embedding_batch_count(32), 1);
        assert_eq!(embedding_batch_count(33), 2);
    }

    fn chunk(content: &str) -> Chunk {
        Chunk {
            page: 1,
            chunk_index: 0,
            content: content.to_string(),
        }
    }

    #[test]
    fn rejects_chunk_and_embedding_input_work_beyond_document_budgets() {
        let chunks = (0..=MAX_DOCUMENT_CHUNKS)
            .map(|_| chunk("x"))
            .collect::<Vec<_>>();
        let error = ensure_chunk_budget(&chunks)
            .expect_err("an unbounded chunk count must not reach embedding");
        assert!(matches!(error, domain::AppError::InvalidInput(_)));

        let input_error = bounded_embedding_input_bytes([{
            MAX_EMBEDDING_INPUT_BYTES - PASSAGE_PREFIX.len() + 1
        }])
        .expect_err("embedding input work beyond its byte budget must be rejected");
        assert!(matches!(input_error, domain::AppError::InvalidInput(_)));
    }

    #[test]
    fn validates_embedding_count_and_exact_vector_dimension_before_arrow_allocation() {
        let chunks = vec![chunk("first"), chunk("second")];
        let count_error = validate_embedding_shapes(&chunks, &[vec![0.0; EMBEDDING_DIMENSION]])
            .expect_err("a model result missing an embedding must be rejected");
        assert!(matches!(count_error, domain::AppError::Internal(_)));

        let dimension_error =
            validate_embedding_shapes(&chunks[..1], &[vec![0.0; EMBEDDING_DIMENSION - 1]])
                .expect_err("a non-768-dimensional vector must be rejected");
        assert!(matches!(dimension_error, domain::AppError::Internal(_)));

        assert_eq!(
            validate_embedding_shapes(&chunks[..1], &[vec![0.0; EMBEDDING_DIMENSION]]).unwrap(),
            EMBEDDING_DIMENSION
        );
        assert!(checked_embedding_value_count(usize::MAX).is_err());
    }
}
