use domain::{ApiKeyStatus, AppError};
use persistence::AppSettingsRepository;

use crate::model_lifecycle::{
    invalidate_external_readiness_before_change, request_llm_initialization_from_persisted_state,
};
use crate::state::AppState;

#[tauri::command]
pub async fn set_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<ApiKeyStatus, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let key_to_save = key.trim().to_string();
    if key_to_save.is_empty() {
        return Err(AppError::invalid_input(
            "Set API key",
            "API key cannot be empty",
            "Enter a valid non-empty API key string.",
        ));
    }

    let _credential_mutation = crate::credentials::MUTATIONS.lock().await;
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let status = crate::credentials::save_api_key(&repo, key_to_save).await?;

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }

    Ok(status)
}

#[tauri::command]
pub async fn get_api_key_status(
    state: tauri::State<'_, AppState>,
) -> Result<ApiKeyStatus, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let _credential_mutation = crate::credentials::MUTATIONS.lock().await;
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let has_keychain_key = tokio::task::spawn_blocking(|| {
        let entry = match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => entry,
            Err(_) => return false,
        };
        match entry.get_password() {
            Ok(password) => !password.is_empty(),
            Err(_) => false,
        }
    })
    .await
    .unwrap_or(false);

    let database_has_key = repo
        .get("external_api_key")
        .await
        .map_err(|_| {
            AppError::storage(
                "Inspect API key storage",
                "The database credential state could not be checked",
                "Restore database access and refresh the API key status.",
            )
        })?
        .is_some_and(|key| !key.trim().is_empty());
    Ok(crate::credentials::storage_status(
        has_keychain_key,
        database_has_key,
    ))
}

#[tauri::command]
pub async fn delete_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let _credential_mutation = crate::credentials::MUTATIONS.lock().await;
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let removed = crate::credentials::remove_api_key(&repo).await?;

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }
    Ok(removed)
}
