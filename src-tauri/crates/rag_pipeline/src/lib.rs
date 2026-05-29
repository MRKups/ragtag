pub mod chunker;
pub mod extractor;

use arrow_array::{FixedSizeListArray, Float32Array, Int32Array, RecordBatch, StringArray};
use domain::{AppError, DocumentStatus};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use persistence::{DocumentRepository, VectorDb};
use std::path::Path;
use std::sync::Arc;

pub use chunker::Chunk;

pub fn init_embedder(cache_dir: &Path) -> Result<TextEmbedding, AppError> {
    std::fs::create_dir_all(cache_dir)?;
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::EmbeddingGemma300M)
            .with_cache_dir(cache_dir.to_path_buf()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to init embedder: {e}")))
}

pub async fn process_document(
    pdf_path: &Path,
    document_id: &str,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    repo: &DocumentRepository,
    vector_db: &VectorDb,
) -> Result<(), AppError> {
    let pages = extractor::extract_pages(pdf_path)?;

    let chunks = chunker::chunk_pages(pages, 512, 80);
    if chunks.is_empty() {
        return Err(AppError::Internal("No text chunks produced".into()));
    }

    repo.update_status(document_id, DocumentStatus::Embedding)
        .await?;

    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();

    let embeddings = {
        let embedder = Arc::clone(&embedder);
        tokio::task::spawn_blocking(move || {
            let mut model = embedder
                .lock()
                .map_err(|e| AppError::Internal(format!("Embedder lock poisoned: {e}")))?;
            model
                .embed(texts.iter().map(|s| s.as_str()).collect::<Vec<_>>(), Some(32))
                .map_err(|e| AppError::Internal(format!("Embedding failed: {e}")))
        })
        .await
        .map_err(|e| AppError::Internal(format!("Embedding task panicked: {e}")))?
    }?;

    let batch = build_record_batch(document_id, &chunks, &embeddings)?;

    vector_db
        .upsert_chunks(std::iter::once(batch))
        .await?;

    repo.update_status(document_id, DocumentStatus::Indexed)
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
    let vector_field = schema.field(0).clone();
    let vector_col = FixedSizeListArray::try_new(
        vector_field.into(),
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

pub fn build_rag_prompt(
    query: &str,
    chunks: &[persistence::SearchResult],
) -> String {
    let mut context = String::from(
        "You are a helpful assistant for querying technical documents. \
        Use ONLY the document excerpts below to answer the user's question. \
        If the excerpts do not contain enough information, say so clearly. \
        Cite sources by indicating the page number in brackets like [Page N].\n\n",
    );

    for chunk in chunks {
        context.push_str(&format!(
            "[Document: {}, Page: {}]\n{}\n\n",
            chunk.document_id, chunk.page, chunk.content
        ));
    }

    context.push_str(&format!("User question: {}", query));

    context
}
