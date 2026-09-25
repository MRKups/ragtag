use domain::{AppError, ExternalProviderConnection};
use persistence::AppSettingsRepository;
use std::collections::HashMap;

use crate::inference::{
    external_model_ids, fetch_external_models, normalize_external_base_url, read_external_api_key,
    DEFAULT_EXTERNAL_MODEL, ExternalInferenceConfig,
};
use crate::model_lifecycle::{
    desired_indexing_provenance, enqueue_requeued_documents,
    invalidate_external_readiness_before_change, invalidate_llm_readiness, parse_chunk_setting,
    request_llm_initialization_from_persisted_state, set_local_model_setting_and_restart_if_active,
    validate_indexing_provenance, validate_setting,
};
use crate::state::AppState;

pub fn inference_mode_is_unchanged(current_mode: Option<&str>, requested_mode: &str) -> bool {
    current_mode.unwrap_or("local") == requested_mode
}

#[tauri::command]
pub async fn get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<HashMap<String, String>, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let mut all = repo.get_all().await?;
    const KEYS: &[&str] = &[
        "inference_mode",
        "external_base_url",
        "external_model",
        "generation_model",
        "tokenizer_model",
        "gguf_filename",
        "use_query_enhancement",
        "chunk_size",
        "chunk_overlap",
    ];
    all.retain(|k, _| KEYS.contains(&k.as_str()));
    Ok(all)
}

#[tauri::command]
pub async fn test_external_provider(
    state: tauri::State<'_, AppState>,
    base_url: String,
    api_key: Option<String>,
) -> Result<ExternalProviderConnection, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let candidate_api_key = api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    let resolved_key = match candidate_api_key {
        Some(key) => Some(key),
        None => read_external_api_key(&repo).await?,
    };
    let config = ExternalInferenceConfig {
        base_url: normalize_external_base_url(&base_url)?,
        model: DEFAULT_EXTERNAL_MODEL.to_string(),
        api_key: resolved_key,
    };
    let models = external_model_ids(&fetch_external_models(&state.http_client, &config).await?);
    if models.is_empty() {
        return Err(AppError::Inference(
            "External provider returned no usable models".into(),
        ));
    }

    Ok(ExternalProviderConnection { models })
}

#[tauri::command]
pub async fn set_external_provider_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    base_url: String,
    model: String,
) -> Result<(), AppError> {
    let normalized_base_url = normalize_external_base_url(&base_url)?;
    validate_setting("external_model", &model)?;
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let write_result = repo
        .set_external_inference_config(normalized_base_url.as_str(), model.trim())
        .await;
    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }
    write_result
}

#[tauri::command]
pub async fn set_setting(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    validate_setting(&key, &value)?;
    if matches!(
        key.as_str(),
        "generation_model" | "tokenizer_model" | "gguf_filename"
    ) {
        return set_local_model_setting_and_restart_if_active(&app, &state, &key, &value).await;
    }

    let repo = AppSettingsRepository::new(state.db_pool.clone());
    if key == "inference_mode" {
        if inference_mode_is_unchanged(repo.get(&key).await?.as_deref(), &value) {
            return Ok(());
        }
        let should_start = invalidate_llm_readiness(&app, &state, true).await;
        let write_result = repo.set(&key, &value).await;
        if should_start {
            request_llm_initialization_from_persisted_state(&app, &state).await?;
        }
        return write_result;
    }

    if matches!(key.as_str(), "external_base_url" | "external_model") {
        let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
        let write_result = repo.set(&key, &value).await;
        if should_start {
            request_llm_initialization_from_persisted_state(&app, &state).await?;
        }
        return write_result;
    }

    let requeued_ids = if matches!(key.as_str(), "chunk_size" | "chunk_overlap") {
        let _document_operation = state.document_operations.write().await;
        let mut desired_provenance = desired_indexing_provenance(&repo).await?;
        match key.as_str() {
            "chunk_size" => {
                desired_provenance.chunk_size = parse_chunk_setting("chunk_size", &value)?;
            }
            "chunk_overlap" => {
                desired_provenance.chunk_overlap = parse_chunk_setting("chunk_overlap", &value)?;
            }
            _ => unreachable!("chunk settings branch only accepts chunk settings"),
        }
        validate_indexing_provenance(&desired_provenance)?;
        repo.set_indexing_setting_and_requeue_stale_documents(&key, &value, &desired_provenance)
            .await?
    } else {
        repo.set(&key, &value).await?;
        Vec::new()
    };

    enqueue_requeued_documents(state.job_queue.clone(), requeued_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_active_inference_mode_as_unchanged() {
        assert!(inference_mode_is_unchanged(Some("local"), "local"));
        assert!(inference_mode_is_unchanged(Some("external"), "external"));
        assert!(inference_mode_is_unchanged(None, "local"));
        assert!(!inference_mode_is_unchanged(Some("local"), "external"));
    }
}
