use domain::AppError;

pub fn storage_sizes(app_local_data: &std::path::Path) -> Result<(u64, u64, u64, u64), AppError> {
    let metadata = std::fs::symlink_metadata(app_local_data)
        .map_err(|error| AppError::Io(format!("Could not inspect app data directory: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }

    Ok((
        managed_file_size(&app_local_data.join("app.db"))?,
        dir_size(&app_local_data.join(".lancedb"))?,
        dir_size(&app_local_data.join("documents"))?,
        dir_size(&app_local_data.join("models"))?,
    ))
}

pub fn managed_file_size(path: &std::path::Path) -> Result<u64, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(AppError::Io(format!(
                "Managed storage path is not a regular file: {}",
                path.display()
            )))
        }
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(AppError::Io(format!(
            "Could not inspect managed storage file {}: {error}",
            path.display()
        ))),
    }
}

pub fn dir_size(path: &std::path::Path) -> Result<u64, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not inspect managed storage directory {}: {error}",
                path.display()
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Io(format!(
            "Managed storage directory is unsafe: {}",
            path.display()
        )));
    }

    let mut total = 0u64;
    let entries = std::fs::read_dir(path).map_err(|error| {
        AppError::Io(format!(
            "Could not read managed storage directory {}: {error}",
            path.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            AppError::Io(format!(
                "Could not inspect an entry in {}: {error}",
                path.display()
            ))
        })?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            AppError::Io(format!(
                "Could not inspect managed storage entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let size = if metadata.is_dir() {
            dir_size(&entry.path())?
        } else if metadata.is_file() {
            metadata.len()
        } else {
            return Err(AppError::Io(format!(
                "Managed storage entry is not a regular file or directory: {}",
                entry.path().display()
            )));
        };
        total = total.checked_add(size).ok_or_else(|| {
            AppError::Io(format!(
                "Managed storage size overflowed for {}",
                path.display()
            ))
        })?;
    }
    Ok(total)
}

pub fn remove_path_if_present(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

pub fn remove_non_fastembed_models(models_dir: &std::path::Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(models_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::other(
                "Managed models directory must be a real directory, not a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let entries = std::fs::read_dir(models_dir)?;

    for entry in entries {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().contains("fastembed") {
            remove_path_if_present(&entry.path())?;
        }
    }
    Ok(())
}

pub fn complete_pending_reset(
    app_local_data: &std::path::Path,
    sentinel: &std::path::Path,
) -> Result<(), AppError> {
    let app_data_metadata = match std::fs::symlink_metadata(app_local_data) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not inspect app data directory: {error}"
            )));
        }
    };
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }
    if std::fs::symlink_metadata(sentinel)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(AppError::Io(
            "Pending reset marker must not be a symlink".into(),
        ));
    }
    let contents = match std::fs::read_to_string(sentinel) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not read pending reset marker: {error}"
            )));
        }
    };
    let config: serde_json::Value = serde_json::from_str(&contents).map_err(|error| {
        AppError::InvalidInput(format!("Pending reset marker is invalid: {error}"))
    })?;
    let delete_models = config["delete_models"].as_bool().unwrap_or(false);

    let mut failures = Vec::new();
    for path in [
        app_local_data.join("documents"),
        app_local_data.join(".lancedb"),
        app_local_data.join("app.db"),
        app_local_data.join("app.db-journal"),
        app_local_data.join("app.db-wal"),
        app_local_data.join("app.db-shm"),
    ] {
        if let Err(error) = remove_path_if_present(&path) {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    if delete_models {
        let models_dir = app_local_data.join("models");
        if let Err(error) = remove_non_fastembed_models(&models_dir) {
            failures.push(format!("{}: {error}", models_dir.display()));
        }
    }

    if !failures.is_empty() {
        return Err(AppError::Io(format!(
            "Pending data reset incomplete: {}",
            failures.join("; "),
        )));
    }

    if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
        let _ = entry.delete_credential();
    }

    std::fs::remove_file(sentinel)
        .map_err(|error| AppError::Io(format!("Could not clear pending reset marker: {error}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::documents::{document_storage_path, managed_storage_filename};

    #[test]
    fn removes_non_fastembed_model_entries() {
        let models_dir =
            std::env::temp_dir().join(format!("ragtag-model-cleanup-{}", uuid::Uuid::now_v7()));
        let other_model = models_dir.join("generation-model");
        let embedder_models = models_dir.join("fastembed");
        std::fs::create_dir_all(&other_model).unwrap();
        std::fs::create_dir_all(&embedder_models).unwrap();
        std::fs::write(other_model.join("model.gguf"), "model").unwrap();
        std::fs::write(embedder_models.join("model.onnx"), "embedder").unwrap();

        remove_non_fastembed_models(&models_dir).unwrap();

        assert!(!other_model.exists());
        assert!(embedder_models.exists());
        std::fs::remove_dir_all(models_dir).unwrap();
    }

    #[test]
    fn reset_removes_the_sqlite_rollback_journal() {
        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-reset-journal-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&app_local_data).unwrap();
        let journal = app_local_data.join("app.db-journal");
        let sentinel = app_local_data.join(".reset_pending");
        std::fs::write(&journal, b"journal").unwrap();
        std::fs::write(&sentinel, r#"{"delete_models":false}"#).unwrap();

        complete_pending_reset(&app_local_data, &sentinel).unwrap();

        assert!(!journal.exists());
        assert!(!sentinel.exists());
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_managed_document_sources_and_models_directory() {
        use std::os::unix::fs::symlink;

        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-symlink-safety-{}", uuid::Uuid::now_v7()));
        let documents = app_local_data.join("documents");
        let outside = app_local_data.join("outside.txt");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        let filename = format!("{}.txt", uuid::Uuid::now_v7());
        symlink(&outside, documents.join(&filename)).unwrap();
        assert!(document_storage_path(&app_local_data, &format!("documents/{filename}")).is_err());
        assert!(managed_storage_filename("documents/../outside.txt").is_err());
        let linked_app_data =
            std::env::temp_dir().join(format!("ragtag-app-data-link-{}", uuid::Uuid::now_v7()));
        symlink(&app_local_data, &linked_app_data).unwrap();
        assert!(document_storage_path(&linked_app_data, &format!("documents/{filename}")).is_err());
        std::fs::remove_file(linked_app_data).unwrap();

        let external_models =
            std::env::temp_dir().join(format!("ragtag-external-models-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&external_models).unwrap();
        symlink(&external_models, app_local_data.join("models")).unwrap();
        assert!(remove_non_fastembed_models(&app_local_data.join("models")).is_err());

        std::fs::remove_dir_all(app_local_data).unwrap();
        std::fs::remove_dir_all(external_models).unwrap();
    }
}
