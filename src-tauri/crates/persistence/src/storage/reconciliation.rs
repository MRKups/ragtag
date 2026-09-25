use domain::{AppError, Document};
use log::{error, info, warn};
use std::collections::HashSet;

use super::documents::{
    document_storage_candidate_for_deletion, is_managed_document_source_filename,
    is_managed_document_temporary_filename, managed_documents_directory, managed_storage_filename,
};
use crate::{DocumentRepository, VectorDb};

pub async fn finish_document_deletion(
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    app_local_data: &std::path::Path,
    document: &Document,
) -> Result<(), AppError> {
    vector_db.delete_chunks(&document.id).await?;

    let check_data = app_local_data.to_path_buf();
    let check_storage_path = document.storage_path.clone();
    let source_candidate = tokio::task::spawn_blocking(move || {
        document_storage_candidate_for_deletion(&check_data, &check_storage_path)
    })
    .await
    .map_err(|error| {
        AppError::internal(
            "Finalize document deletion",
            format!("Document storage candidate task panicked: {error}"),
            "Restart Ragtag to retry pending document deletion cleanup.",
        )
    })??;

    if let Some(source_path) = source_candidate {
        match tokio::fs::symlink_metadata(&source_path).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                tokio::fs::remove_file(source_path)
                    .await
                    .map_err(|error| {
                        AppError::storage(
                            "Delete document file",
                            error.to_string(),
                            "Verify filesystem permissions in the documents folder.",
                        )
                    })?;
            }
            Ok(_) => {
                return Err(AppError::storage(
                    "Delete document file",
                    "Managed document source must be a regular file, not a symlink",
                    "Verify file structure integrity in the documents folder.",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::storage(
                    "Delete document file",
                    error.to_string(),
                    "Verify filesystem permissions in the documents folder.",
                ));
            }
        }
    }

    repo.delete_by_id(&document.id).await
}

pub async fn reconcile_pending_deletions(
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    app_local_data: &std::path::Path,
) {
    let documents = match repo.get_deleting_documents().await {
        Ok(documents) => documents,
        Err(error) => {
            error!("startup deletion recovery — failed to list deleting documents: {error}");
            return;
        }
    };

    for document in documents {
        let document_id = document.id.clone();
        if let Err(error) =
            finish_document_deletion(repo, vector_db, app_local_data, &document).await
        {
            error!("startup deletion recovery — failed to delete document {document_id}: {error}");
        } else {
            info!("startup deletion recovery — deleted document {document_id}");
        }
    }
}

pub async fn reconcile_orphan_document_sources(
    repo: &DocumentRepository,
    app_local_data: &std::path::Path,
) -> Result<usize, AppError> {
    let registered_filenames: HashSet<String> = repo
        .list_all()
        .await?
        .into_iter()
        .filter_map(|document| {
            managed_storage_filename(&document.storage_path)
                .ok()
                .map(str::to_owned)
        })
        .collect();
    let untrusted_docs_dir = app_local_data.join("documents");
    match tokio::fs::symlink_metadata(&untrusted_docs_dir).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(AppError::storage(
                "Inspect documents directory",
                error.to_string(),
                "Verify folder permissions in application local data.",
            ));
        }
    }
    let docs_dir = tokio::task::spawn_blocking({
        let app_local_data = app_local_data.to_path_buf();
        move || managed_documents_directory(&app_local_data, false)
    })
    .await
    .map_err(|error| {
        AppError::internal(
            "Reconcile document sources",
            format!("Managed document directory task panicked: {error}"),
            "Restart Ragtag to reconcile document files.",
        )
    })??;
    let mut entries = tokio::fs::read_dir(&docs_dir)
        .await
        .map_err(|error| {
            AppError::storage(
                "Read documents directory",
                error.to_string(),
                "Verify folder permissions in application local data.",
            )
        })?;
    let mut removed = 0;

    while let Some(entry) = entries.next_entry().await? {
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let is_temporary = is_managed_document_temporary_filename(filename);
        if (!is_temporary && !is_managed_document_source_filename(filename))
            || (!is_temporary && registered_filenames.contains(filename))
        {
            continue;
        }

        match entry.file_type().await {
            Ok(file_type) if file_type.is_file() => {}
            Ok(_) => continue,
            Err(error) => {
                warn!("startup source reconciliation — could not inspect {filename}: {error}");
                continue;
            }
        }

        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(
                    "startup source reconciliation — could not remove orphan {filename}: {error}"
                );
            }
        }
    }

    Ok(removed)
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
            file_hash: Some(vec![1, 2, 3]),
            storage_path: format!("documents/{filename}"),
            status: domain::DocumentStatus::Indexed,
            chunk_count: Some(1),
            embedding_model: Some("test-model".into()),
            embedding_attempts: 1,
            created_at: None,
            processing_started_at: None,
            indexed_at: None,
            indexing_provenance: None,
            error_message: None,
            retry_count: 0,
        }
    }

    #[tokio::test]
    async fn removes_only_unregistered_managed_document_sources() {
        let app_local_data = std::env::temp_dir().join(format!(
            "ragtag-source-reconciliation-{}",
            uuid::Uuid::now_v7()
        ));
        let docs_dir = app_local_data.join("documents");
        tokio::fs::create_dir_all(&docs_dir).await.unwrap();
        let pool = crate::init_pool(&app_local_data).await.unwrap();
        let repo = crate::DocumentRepository::new(pool.clone());
        let registered_filename = format!("{}.txt", uuid::Uuid::now_v7());
        let orphan_filename = format!("{}.txt", uuid::Uuid::now_v7());
        let managed_directory_name = format!("{}.txt", uuid::Uuid::now_v7());
        let mut registered = indexed_text_document("registered.txt");
        registered.storage_path = format!("documents/{registered_filename}");
        repo.insert(&registered).await.unwrap();
        tokio::fs::write(docs_dir.join(&registered_filename), b"registered")
            .await
            .unwrap();
        tokio::fs::write(docs_dir.join(&orphan_filename), b"orphan")
            .await
            .unwrap();
        tokio::fs::write(docs_dir.join("user-note.txt"), b"not managed")
            .await
            .unwrap();
        let temporary_filename = format!(".{orphan_filename}.{}.tmp", uuid::Uuid::now_v7());
        tokio::fs::write(docs_dir.join(&temporary_filename), b"partial")
            .await
            .unwrap();
        tokio::fs::create_dir(docs_dir.join(&managed_directory_name))
            .await
            .unwrap();

        assert_eq!(
            reconcile_orphan_document_sources(&repo, &app_local_data)
                .await
                .unwrap(),
            2
        );
        assert!(docs_dir.join(registered_filename).exists());
        assert!(!docs_dir.join(orphan_filename).exists());
        assert!(docs_dir.join("user-note.txt").exists());
        assert!(!docs_dir.join(temporary_filename).exists());
    }
}
