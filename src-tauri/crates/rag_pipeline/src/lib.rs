pub mod chunker;
pub mod extractor;

use arrow_array::{FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray};
use domain::AppError;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use persistence::{DocumentRepository, VectorDb};
use std::path::Path;
use std::sync::Arc;

pub use chunker::Chunk;

const EMBEDDING_MAX_TOKENS: usize = 2_048;

pub fn init_embedder(cache_dir: &Path) -> Result<TextEmbedding, AppError> {
    std::fs::create_dir_all(cache_dir)?;
    TextEmbedding::try_new(
        TextInitOptions::new(EmbeddingModel::EmbeddingGemma300M)
            .with_max_length(EMBEDDING_MAX_TOKENS)
            .with_cache_dir(cache_dir.to_path_buf()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to init embedder: {e}")))
}

pub async fn process_document(
    pdf_path: &Path,
    document_id: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    on_progress: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(), AppError> {
    repo.mark_embedding(document_id).await?;

    on_progress("extracting");
    let pdf_path = pdf_path.to_path_buf();
    let pages = tokio::task::spawn_blocking(move || extractor::extract_pages(&pdf_path))
        .await
        .map_err(|e| AppError::Internal(format!("Extraction task panicked: {e}")))??;

    on_progress("chunking");
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
    let chunks = tokio::task::spawn_blocking(move || {
        let chunks = chunker::chunk_pages(
            pages,
            embedding_token_limit.min(chunk_size),
            chunk_overlap,
            tokenizer,
        )?;
        if chunks.is_empty() {
            return Err(AppError::Internal("No text chunks produced".into()));
        }
        Ok(chunks)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Chunking task panicked: {e}")))??;

    on_progress("embedding");

    let (chunks, embeddings) = {
        let embedder = Arc::clone(&embedder);
        tokio::task::spawn_blocking(move || {
            let mut model = embedder
                .lock()
                .map_err(|e| AppError::Internal(format!("Embedder lock poisoned: {e}")))?;
            let texts = chunks
                .iter()
                .map(|chunk| format!("passage: {}", chunk.content))
                .collect::<Vec<_>>();
            let embeddings = model
                .embed(
                    texts.iter().map(String::as_str).collect::<Vec<_>>(),
                    Some(32),
                )
                .map_err(|e| AppError::Internal(format!("Embedding failed: {e}")))?;
            Ok::<_, AppError>((chunks, embeddings))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Embedding task panicked: {e}")))?
    }?;

    let batch = build_record_batch(document_id, &chunks, &embeddings)?;

    on_progress("storing");
    vector_db.delete_chunks(document_id).await?;
    vector_db.upsert_chunks(std::iter::once(batch)).await?;

    repo.mark_indexed(document_id, chunks.len(), "EmbeddingGemma-300M")
        .await?;

    Ok(())
}

fn build_record_batch(
    document_id: &str,
    chunks: &[Chunk],
    embeddings: &[Vec<f32>],
) -> Result<RecordBatch, AppError> {
    let schema = VectorDb::schema();
    let num_chunks = chunks.len();
    let dim: usize = 768;

    let mut all_floats: Vec<f32> = Vec::with_capacity(num_chunks * dim);
    let mut doc_ids: Vec<String> = Vec::with_capacity(num_chunks);
    let mut pages: Vec<i32> = Vec::with_capacity(num_chunks);
    let mut chunk_indices: Vec<i32> = Vec::with_capacity(num_chunks);
    let mut contents: Vec<String> = Vec::with_capacity(num_chunks);

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
        dim as i32,
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
    embeddings
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal("No embedding returned".into()))
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
        {
            if !expansions.contains(expansion) {
                expansions.push(*expansion);
            }
        }
    }

    if expansions.is_empty() {
        query.to_string()
    } else {
        format!("{query} ({})", expansions.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::enhance_query;

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
}

pub fn build_rag_prompt(query: &str, chunks: &[domain::Citation]) -> String {
    let mut context = String::new();

    for chunk in chunks {
        context.push_str(&format!(
            "[Document: {}, Page: {}]\n{}\n\n",
            chunk.filename, chunk.page, chunk.chunk_content
        ));
    }

    context.push_str(&format!("User question: {}", query));

    context
}
