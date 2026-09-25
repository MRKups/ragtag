use domain::{AppError, Document, DocumentPreview, DocumentStatus, IngestionProgress};
use log::warn;
use persistence::DocumentRepository;
use sha2::{Digest, Sha512};
use tauri::{Emitter, Manager};

use crate::state::{AppState, IngestionJob};
use persistence::storage::{
    document_storage_candidate_for_deletion, document_storage_path, finish_document_deletion,
    managed_documents_directory, storage_extension, validate_upload,
    write_document_source_atomically,
};

pub fn validate_document_preview_document(document: &Document) -> Result<(), AppError> {
    if document.status != DocumentStatus::Indexed {
        return Err(AppError::NotFound(
            "Document is not available for preview".into(),
        ));
    }
    if !matches!(document.file_type.as_str(), "text" | "docx") {
        return Err(AppError::InvalidInput(
            "A preview is unavailable for this document type".into(),
        ));
    }

    Ok(())
}

#[tauri::command]
pub async fn upload_document(
    app: tauri::AppHandle,
    filename: String,
    file_bytes: Vec<u8>,
    channel: tauri::ipc::Channel<IngestionProgress>,
) -> Result<String, AppError> {
    let file_type = validate_upload(&filename, &file_bytes)?;
    let state = app.state::<AppState>();
    if state.embedder.read().await.is_none() {
        return Err(AppError::Internal(
            "Embedding model not yet loaded. Please wait before uploading documents.".into(),
        ));
    }
    let repo = DocumentRepository::new(state.db_pool.clone());

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let docs_dir = tokio::task::spawn_blocking({
        let app_local_data = app_local_data.clone();
        move || managed_documents_directory(&app_local_data, true)
    })
    .await
    .map_err(|error| {
        AppError::Internal(format!("Managed document directory task panicked: {error}"))
    })??;

    let file_size_bytes = i64::try_from(file_bytes.len())
        .map_err(|_| AppError::InvalidInput("Document is too large to store".into()))?;
    let (hash, file_bytes) = tokio::task::spawn_blocking(move || {
        let mut hasher = Sha512::new();
        hasher.update(&file_bytes);
        (hasher.finalize().to_vec(), file_bytes)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Hashing failed: {e}")))?;

    if let Some(existing) = repo.find_by_hash(&hash).await? {
        return Err(AppError::DuplicateDocument(format!(
            "This file was already uploaded as '{}' (status: {:?})",
            existing.filename, existing.status
        )));
    }

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.{}", doc_id, storage_extension(&filename, file_type));
    let storage_path = docs_dir.join(&storage_filename);
    let relative_storage_path = format!("documents/{}", storage_filename);

    tokio::task::spawn_blocking({
        let storage_path = storage_path.clone();
        move || write_document_source_atomically(&storage_path, &file_bytes)
    })
    .await
    .map_err(|error| AppError::Internal(format!("Document write task panicked: {error}")))??;

    let created_at = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string();

    let doc = Document {
        id: doc_id.clone(),
        filename,
        file_type: file_type.to_string(),
        file_size_bytes: Some(file_size_bytes),
        file_hash: Some(hash),
        storage_path: relative_storage_path,
        status: DocumentStatus::Queued,
        chunk_count: None,
        embedding_model: None,
        embedding_attempts: 0,
        created_at: Some(created_at),
        processing_started_at: None,
        indexed_at: None,
        indexing_provenance: None,
        error_message: None,
        retry_count: 0,
    };

    if let Err(e) = repo.insert(&doc).await {
        let _ = tokio::fs::remove_file(&storage_path).await;
        return Err(e);
    }

    if state
        .job_queue
        .try_send(IngestionJob {
            document_id: doc_id.clone(),
            progress_channel: Some(channel),
        })
        .is_err()
    {
        if let Err(cleanup_error) = repo.delete_by_id(&doc_id).await {
            warn!(
                "queue-full upload rollback could not delete document {doc_id}; scheduling durable enqueue: {cleanup_error}"
            );
            let queue = state.job_queue.clone();
            let queued_document_id = doc_id.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(send_error) = queue
                    .send(IngestionJob::background(queued_document_id))
                    .await
                {
                    log::error!("queue-full upload recovery producer stopped: {send_error}");
                }
            });
            return Err(AppError::QueueFull);
        }
        let _ = tokio::fs::remove_file(&storage_path).await;
        return Err(AppError::QueueFull);
    }

    Ok(doc_id)
}

#[tauri::command]
pub async fn list_documents(state: tauri::State<'_, AppState>) -> Result<Vec<Document>, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    repo.list_all().await
}

#[tauri::command]
pub async fn get_document_preview(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    document_id: String,
) -> Result<DocumentPreview, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    let document = repo
        .get_by_id(&document_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Document is not available for preview".into()))?;
    validate_document_preview_document(&document)?;

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let storage_path = document.storage_path;
    let file_type = document.file_type;
    let filename = document.filename;

    tokio::task::spawn_blocking(move || {
        let source_path = document_storage_path(&app_local_data, &storage_path)?;
        rag_pipeline::extractor::document_preview(&source_path, &file_type, &filename)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Document preview task panicked: {e}")))?
}

#[tauri::command]
pub async fn delete_document(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::InvalidInput("Document ID must be a UUID".into()))?
        .to_string();
    let _document_operation = state.document_operations.write().await;
    let repo = DocumentRepository::new(state.db_pool.clone());
    let document = repo.get_by_id(&id).await?;
    if let Some(document) = document {
        let app_local_data = app
            .path()
            .app_local_data_dir()
            .map_err(|error| AppError::Io(error.to_string()))?;
        let check_data = app_local_data.clone();
        let check_storage_path = document.storage_path.clone();
        tokio::task::spawn_blocking(move || {
            document_storage_candidate_for_deletion(&check_data, &check_storage_path)
        })
        .await
        .map_err(|error| {
            AppError::Internal(format!("Document storage check task panicked: {error}"))
        })??;
        repo.mark_deleting(&id).await?;
        finish_document_deletion(&repo, &state.vector_db, &app_local_data, &document).await?;
    } else {
        state.vector_db.delete_chunks(&id).await?;
    }
    let _ = app.emit(
        "document_deleted",
        serde_json::json!({
            "document_id": &id,
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn authorizes_only_indexed_text_and_docx_documents_for_preview() {
        let mut document = indexed_text_document("notes.txt");
        assert!(validate_document_preview_document(&document).is_ok());

        document.filename = "notes.md".into();
        assert!(validate_document_preview_document(&document).is_ok());

        document.file_type = "docx".into();
        document.filename = "manual.docx".into();
        assert!(validate_document_preview_document(&document).is_ok());

        document.file_type = "pdf".into();
        assert!(matches!(
            validate_document_preview_document(&document),
            Err(AppError::InvalidInput(_))
        ));

        document.status = DocumentStatus::Embedding;
        assert!(matches!(
            validate_document_preview_document(&document),
            Err(AppError::NotFound(_))
        ));
    }
}
