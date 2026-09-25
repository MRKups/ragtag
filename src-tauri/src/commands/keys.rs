use domain::{ApiKeyStatus, ApiKeyStorage, AppError};
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
    let key_to_save = key.trim().to_string();
    if key_to_save.is_empty() {
        return Err(AppError::InvalidInput("API key cannot be empty".into()));
    }

    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let key_for_keychain = key_to_save.clone();
    let keychain_result = tokio::task::spawn_blocking(move || {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| format!("Entry error: {e}"))?;
        entry.set_password(&key_for_keychain)
            .map_err(|e| format!("Set password error: {e}"))?;
        Ok::<(), String>(())
    })
    .await;

    let storage = match keychain_result {
        Ok(Ok(())) => {
            let _ = repo.delete("external_api_key").await;
            ApiKeyStorage::Keychain
        }
        _ => {
            log::warn!("System keychain is unavailable; falling back to SQLite storage for API key");
            repo.set("external_api_key", &key_to_save).await?;
            ApiKeyStorage::Database
        }
    };

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }

    Ok(ApiKeyStatus {
        has_key: true,
        storage,
    })
}

#[tauri::command]
pub async fn get_api_key_status(
    state: tauri::State<'_, AppState>,
) -> Result<ApiKeyStatus, AppError> {
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

    if has_keychain_key {
        return Ok(ApiKeyStatus {
            has_key: true,
            storage: ApiKeyStorage::Keychain,
        });
    }

    if let Ok(Some(db_key)) = repo.get("external_api_key").await
        && !db_key.trim().is_empty()
    {
        return Ok(ApiKeyStatus {
            has_key: true,
            storage: ApiKeyStorage::Database,
        });
    }

    Ok(ApiKeyStatus {
        has_key: false,
        storage: ApiKeyStorage::None,
    })
}

#[tauri::command]
pub async fn delete_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, AppError> {
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
            let _ = entry.delete_credential();
        }
    })
    .await;

    let _ = repo.delete("external_api_key").await;

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }
    Ok(true)
}
