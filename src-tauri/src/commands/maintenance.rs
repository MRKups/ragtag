use domain::AppError;
use log::info;
use tauri::Manager;

use crate::state::AppState;
use persistence::storage::storage_sizes;

#[tauri::command]
pub async fn get_storage_info(
    app: tauri::AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| {
            AppError::storage(
                "Resolve storage path",
                format!("Failed to resolve app local data directory: {e}"),
                "Verify application local storage permissions.",
            )
        })?;

    let (db_size, lancedb_size, docs_size, models_size) =
        tokio::task::spawn_blocking(move || storage_sizes(&app_local_data))
            .await
            .map_err(|error| {
                AppError::internal(
                    "Inspect storage sizes",
                    format!("Storage accounting task panicked: {error}"),
                    "Restart the application to refresh storage metrics.",
                )
            })??;

    Ok(serde_json::json!({
        "database_bytes": db_size,
        "vector_store_bytes": lancedb_size,
        "documents_bytes": docs_size,
        "models_bytes": models_size,
    }))
}

#[tauri::command]
pub async fn compact_vector_store(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    state.vector_db.compact().await?;
    Ok(serde_json::json!({"status": "compacted"}))
}

#[tauri::command]
pub async fn vacuum_database(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    persistence::vacuum(&state.db_pool).await?;
    Ok(serde_json::json!({"status": "vacuumed"}))
}

#[tauri::command]
pub async fn reset_all_data(
    app: tauri::AppHandle,
    delete_models: bool,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| {
            AppError::storage(
                "Resolve storage path",
                format!("Failed to resolve app local data directory: {e}"),
                "Verify application local storage permissions.",
            )
        })?;

    let sentinel = app_local_data.join(".reset_pending");
    tokio::task::spawn_blocking({
        let app_local_data = app_local_data.clone();
        let sentinel = sentinel.clone();
        move || {
            let metadata = std::fs::symlink_metadata(&app_local_data).map_err(|error| {
                AppError::storage(
                    "Inspect data directory",
                    format!("Could not inspect app data directory: {error}"),
                    "Ensure the application data directory is accessible.",
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::invalid_input(
                    "Validate data directory",
                    "App data directory must be a real directory, not a symlink",
                    "Use a regular directory for application local data.",
                ));
            }
            if std::fs::symlink_metadata(&sentinel)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(AppError::invalid_input(
                    "Validate reset marker",
                    "Pending reset marker must not be a symlink",
                    "Remove the existing invalid reset marker file before retrying reset.",
                ));
            }
            Ok(())
        }
    })
    .await
    .map_err(|error| {
        AppError::internal(
            "Validate reset marker",
            format!("Reset validation task panicked: {error}"),
            "Restart the application and retry data reset.",
        )
    })??;
    let payload = serde_json::json!({"delete_models": delete_models});
    tokio::fs::write(&sentinel, payload.to_string()).await?;

    if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
        let _ = entry.delete_credential();
    }

    info!("reset requested — restarting application");
    app.request_restart();

    Ok(serde_json::json!({"restarting": true}))
}
