use domain::AppError;
use log::info;
use tauri::Manager;

use crate::state::AppState;
use persistence::storage::storage_sizes;

#[tauri::command]
pub async fn get_storage_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let app_local_data = app.path().app_local_data_dir().map_err(|e| {
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
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    state.vector_db.compact().await?;
    Ok(serde_json::json!({"status": "compacted"}))
}

#[tauri::command]
pub async fn vacuum_database(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    persistence::vacuum(&state.db_pool).await?;
    Ok(serde_json::json!({"status": "vacuumed"}))
}

#[tauri::command]
pub async fn reset_all_data(
    app: tauri::AppHandle,
    delete_models: bool,
) -> Result<serde_json::Value, AppError> {
    let state = app.state::<AppState>();
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let _doc_op = state.document_operations.write().await;
    let app_local_data = app.path().app_local_data_dir().map_err(|e| {
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
    // Do not schedule a destructive reset until credential cleanup succeeds.
    let settings = persistence::AppSettingsRepository::new(state.db_pool.clone());
    let _credential_mutation = crate::credentials::MUTATIONS.lock().await;
    prepare_reset_marker(
        &sentinel,
        delete_models,
        crate::credentials::remove_api_key(&settings),
    )
    .await?;

    info!("reset requested — restarting application");
    app.request_restart();

    Ok(serde_json::json!({"restarting": true}))
}

async fn prepare_reset_marker(
    sentinel: &std::path::Path,
    delete_models: bool,
    credential_cleanup: impl std::future::Future<Output = Result<bool, AppError>>,
) -> Result<(), AppError> {
    credential_cleanup.await?;
    let payload = serde_json::json!({"delete_models": delete_models});
    tokio::fs::write(sentinel, payload.to_string()).await?;
    Ok(())
}

#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), AppError> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|e| {
        AppError::invalid_input(
            "Open external URL",
            format!("Invalid URL: {e}"),
            "Provide a valid URL.",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err(AppError::invalid_input(
            "Open external URL",
            "Only HTTP, HTTPS, and mailto URLs may be opened",
            "Provide an HTTP, HTTPS, or mailto link.",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::invalid_input(
            "Open external URL",
            "URLs with embedded credentials are not permitted",
            "Provide a standard URL without username or password.",
        ));
    }
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let res = std::process::Command::new("open")
            .arg(parsed.as_str())
            .spawn();
        #[cfg(target_os = "windows")]
        let res = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", parsed.as_str()])
            .spawn();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let res = std::process::Command::new("xdg-open")
            .arg(parsed.as_str())
            .spawn();

        res.map_err(|error| {
            AppError::internal(
                "Open external URL",
                format!("Failed to spawn browser process: {error}"),
                "Ensure your operating system has a default web browser configured.",
            )
        })
    })
    .await
    .map_err(|e| {
        AppError::internal(
            "Open external URL",
            format!("Browser launcher task panicked: {e}"),
            "Restart the application and try opening the link again.",
        )
    })??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::ErrorCode;

    #[tokio::test]
    async fn failed_credential_cleanup_does_not_schedule_a_reset() {
        let dir = std::env::temp_dir().join(format!("ragtag-reset-order-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let sentinel = dir.join(".reset_pending");
        let cleanup = async {
            Err(AppError::storage(
                "Delete API key",
                "Test keychain unavailable",
                "Retry credential cleanup.",
            ))
        };
        let error = prepare_reset_marker(&sentinel, true, cleanup)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(!tokio::fs::try_exists(&sentinel).await.unwrap());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn successful_cleanup_writes_the_reset_marker_with_model_choice() {
        let dir =
            std::env::temp_dir().join(format!("ragtag-reset-success-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let sentinel = dir.join(".reset_pending");
        prepare_reset_marker(&sentinel, false, async { Ok(false) })
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&sentinel).await.unwrap()).unwrap();
        assert_eq!(payload, serde_json::json!({"delete_models":false}));
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_and_unsafe_external_urls() {
        let err = open_external_url("not a url".to_string())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let err = open_external_url("javascript:alert(1)".to_string())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let err = open_external_url("file:///etc/passwd".to_string())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let err = open_external_url("data:text/html,bad".to_string())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);

        let err = open_external_url("http://user:pass@example.com".to_string())
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }
}
