//! Model lifecycle coordination and settings reconciliation.
//!
//! Manages model downloading, background initialization, external provider validation,
//! and generation-tracked settings updates. Rapid setting changes coalesce via monotonic
//! generation counters so only the latest configuration triggers active model reload.

use domain::AppError;
use log::{error, info, warn};
use persistence::AppSettingsRepository;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;

use crate::inference::external::{
    external_inference_config, normalize_external_base_url, probe_external_inference_readiness,
};
use crate::model_download::{self, DownloadStage, ModelDownloadProgress, ProgressEmitter};
use crate::state::{AppState, LlmInitializationState};

pub const DEFAULT_TOKENIZER_MODEL: &str = "Qwen/Qwen3-1.7B";

pub fn configure_local_generation(
    builder: mistralrs::GgufModelBuilder,
) -> mistralrs::GgufModelBuilder {
    // v0.9.5 enables paged Metal attention by default. It stalls on this app's
    // default GGUF; eager Metal generation retains engine-managed concurrency.
    #[cfg(target_os = "macos")]
    let builder = builder.without_paged_attn();
    builder
}

pub fn is_valid_hugging_face_model_id(value: &str) -> bool {
    value.split_once('/').is_some_and(|(organization, model)| {
        !organization.is_empty()
            && !model.is_empty()
            && !model.contains('/')
            && !value.contains(char::is_whitespace)
    })
}

fn is_valid_gguf_filename(value: &str) -> bool {
    !value.is_empty() && !value.contains('/') && !value.contains('\\') && value.ends_with(".gguf")
}

pub fn validate_setting(key: &str, value: &str) -> Result<(), AppError> {
    match key {
        "theme" if matches!(value, "light" | "dark") => Ok(()),
        "inference_mode" if matches!(value, "local" | "external") => Ok(()),
        "use_query_enhancement" if matches!(value, "true" | "false") => Ok(()),
        "chunk_size" => value
            .parse::<usize>()
            .ok()
            .filter(|size| (128..=2048).contains(size))
            .map(|_| ())
            .ok_or_else(|| {
                AppError::invalid_input(
                    "Validate setting",
                    "chunk_size must be 128–2048",
                    "Set chunk_size to an integer between 128 and 2048.",
                )
            }),
        "chunk_overlap" => value
            .parse::<usize>()
            .ok()
            .filter(|size| *size <= 1024)
            .map(|_| ())
            .ok_or_else(|| {
                AppError::invalid_input(
                    "Validate setting",
                    "chunk_overlap must be 0–1024",
                    "Set chunk_overlap to an integer between 0 and 1024.",
                )
            }),
        "external_base_url" => normalize_external_base_url(value).map(|_| ()),
        "external_model" if !value.trim().is_empty() => Ok(()),

        "generation_model" if is_valid_hugging_face_model_id(value) => Ok(()),
        "tokenizer_model" if is_valid_hugging_face_model_id(value) => Ok(()),
        "gguf_filename" if is_valid_gguf_filename(value) => Ok(()),
        _ => Err(AppError::invalid_input(
            "Validate setting",
            format!("Unsupported or invalid setting: {key}"),
            "Provide a supported setting name and valid configuration value.",
        )),
    }
}

pub fn invalidate_llm_initialization(initialization: &mut LlmInitializationState) {
    initialization.generation = initialization.generation.wrapping_add(1);
}

// Only the task that owns the active generation may clear it. Callers retain
// their distinct current/stale handling and decide whether to restart.
fn finish_owned_llm_initialization(
    current: &mut LlmInitializationState,
    generation: u64,
) -> Option<bool> {
    if current.in_progress_generation != Some(generation) {
        return None;
    }

    current.in_progress_generation = None;
    current.download_cancellation = None;
    Some(std::mem::take(&mut current.restart_requested))
}

fn publish_model_download_progress(
    app: &tauri::AppHandle,
    current: &mut LlmInitializationState,
    progress: ModelDownloadProgress,
) {
    current.last_download_progress = Some(progress.clone());
    let _ = app.emit("model_download_progress", progress);
}

fn model_error_progress(model_id: String, message: String) -> ModelDownloadProgress {
    ModelDownloadProgress {
        model_id,
        current_file: String::new(),
        file_downloaded: 0,
        file_total: 0,
        files_completed: 0,
        total_files: 0,
        stage: DownloadStage::Error { message },
    }
}

pub async fn finish_llm_initialization(
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
) -> bool {
    let mut current = initialization.lock().await;
    let Some(restart) = finish_owned_llm_initialization(&mut current, generation) else {
        return false;
    };

    if current.generation == generation {
        return false;
    }

    restart
}

pub async fn llm_initialization_is_current(
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
) -> bool {
    let current = initialization.lock().await;
    current.generation == generation && current.in_progress_generation == Some(generation)
}

pub async fn emit_model_download_progress_if_current(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
    progress: ModelDownloadProgress,
) -> bool {
    let mut current = initialization.lock().await;
    if current.generation != generation || current.in_progress_generation != Some(generation) {
        return false;
    }

    publish_model_download_progress(app, &mut current, progress);
    true
}

pub async fn finish_llm_initialization_with_progress(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
    progress: ModelDownloadProgress,
) -> bool {
    let mut current = initialization.lock().await;
    let Some(restart) = finish_owned_llm_initialization(&mut current, generation) else {
        return false;
    };

    if current.generation != generation {
        return restart;
    }

    publish_model_download_progress(app, &mut current, progress);
    false
}

pub async fn finish_external_readiness_probe(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
    readiness: Result<(), AppError>,
) -> bool {
    let mut current = initialization.lock().await;
    let Some(restart) = finish_owned_llm_initialization(&mut current, generation) else {
        return false;
    };

    if current.generation != generation {
        return restart;
    }

    match readiness {
        Ok(()) => {
            current.external_ready = true;
            info!("external inference endpoint and model are ready");
            let _ = app.emit("llm_ready", ());
        }
        Err(error) => {
            current.external_ready = false;
            warn!("external inference readiness check failed: {error}");
            let _ = app.emit("external_inference_failed", error);
        }
    }
    false
}

pub async fn finish_local_model_initialization(
    app: &tauri::AppHandle,
    llm_state: &Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: &Arc<tokio::sync::Mutex<LlmInitializationState>>,
    generation: u64,
    model: mistralrs::Model,
) -> bool {
    let restart = {
        let mut current = initialization.lock().await;
        let Some(restart) = finish_owned_llm_initialization(&mut current, generation) else {
            return false;
        };

        if current.generation != generation {
            return restart;
        }

        restart
    };

    let mut write_guard = llm_state.write().await;
    let current = initialization.lock().await;
    if current.generation == generation {
        *write_guard = Some(Arc::new(model));
        drop(write_guard);
        drop(current);
        info!("mistral.rs pipeline built and ready");
        let _ = app.emit("llm_ready", ());
        restart
    } else {
        drop(write_guard);
        let restart = restart || current.restart_requested;
        drop(current);
        restart
    }
}

pub async fn invalidate_llm_readiness(
    app: &tauri::AppHandle,
    state: &AppState,
    clear_local_model: bool,
) -> bool {
    let should_start = {
        let mut initialization = state.llm_initialization.lock().await;
        invalidate_llm_initialization(&mut initialization);
        if let Some(cancel) = initialization.download_cancellation.take() {
            let _ = cancel.send(true);
        }
        initialization.external_ready = false;
        let should_start = !initialization.is_in_progress();
        if !should_start {
            initialization.restart_requested = true;
        }
        should_start
    };

    if clear_local_model {
        *state.local_llm.write().await = None;
    }
    let _ = app.emit("llm_unready", ());
    should_start
}

pub async fn cancel_active_model_download(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let mut current = state.llm_initialization.lock().await;
    if current.is_in_progress() {
        if let Some(cancel) = current.download_cancellation.take() {
            let _ = cancel.send(true);
        }
        invalidate_llm_initialization(&mut current);
        current.in_progress_generation = None;
        current.restart_requested = false;
        current.last_download_progress = None;
        let _ = app.emit("model_download_cancelled", ());
        let _ = app.emit("llm_unready", ());
    }
    Ok(())
}

pub async fn invalidate_external_readiness_before_change(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let mode = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    if mode == "external" {
        return Ok(invalidate_llm_readiness(app, state, false).await);
    }

    Ok(false)
}

pub async fn set_local_model_setting_and_restart_if_active(
    app: &tauri::AppHandle,
    state: &AppState,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    if settings.get(key).await?.as_deref() == Some(value) {
        return Ok(());
    }

    let local_mode_active = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string())
        == "local";
    let should_start = local_mode_active && invalidate_llm_readiness(app, state, true).await;
    let write_result = settings.set(key, value).await;
    if should_start {
        request_llm_initialization_from_persisted_state(app, state).await?;
    }
    write_result
}

pub async fn request_llm_initialization_from_persisted_state(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let models_dir =
        crate::paths::models_dir(app, "Ensure application storage path is accessible.")?;
    request_llm_initialization(app, state, models_dir).await;
    Ok(())
}

pub async fn request_llm_initialization(
    app: &tauri::AppHandle,
    state: &AppState,
    models_dir: PathBuf,
) {
    request_llm_initialization_with_resources(
        app.clone(),
        state.local_llm.clone(),
        state.llm_initialization.clone(),
        state.db_pool.clone(),
        state.http_client.clone(),
        models_dir,
    )
    .await;
}

pub fn schedule_llm_initialization_with_resources(
    app: tauri::AppHandle,
    llm_state: Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: Arc<tokio::sync::Mutex<LlmInitializationState>>,
    pool: sqlx::SqlitePool,
    http_client: reqwest::Client,
    models_dir: PathBuf,
) {
    tauri::async_runtime::spawn(async move {
        request_llm_initialization_with_resources(
            app,
            llm_state,
            initialization,
            pool,
            http_client,
            models_dir,
        )
        .await;
    });
}

pub async fn request_llm_initialization_with_resources(
    app: tauri::AppHandle,
    llm_state: Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: Arc<tokio::sync::Mutex<LlmInitializationState>>,
    pool: sqlx::SqlitePool,
    http_client: reqwest::Client,
    models_dir: PathBuf,
) {
    let (generation, cancel_rx) = {
        let mut current = initialization.lock().await;
        if current.is_in_progress() {
            return;
        }
        current.in_progress_generation = Some(current.generation);
        current.restart_requested = false;
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        current.download_cancellation = Some(cancel_tx);
        current.last_download_progress = None;
        (current.generation, cancel_rx)
    };

    tauri::async_runtime::spawn(async move {
        let schedule_restart = || {
            schedule_llm_initialization_with_resources(
                app.clone(),
                llm_state.clone(),
                initialization.clone(),
                pool.clone(),
                http_client.clone(),
                models_dir.clone(),
            );
        };
        let app_settings = AppSettingsRepository::new(pool.clone());
        let mode = match app_settings.get("inference_mode").await {
            Ok(mode) => {
                let mode = mode.unwrap_or_else(|| "local".to_string());
                if let Ok(mut current) = initialization.try_lock() {
                    current.inference_mode = mode.clone();
                }
                mode
            }
            Err(error) => {
                let message = format!("Failed to read inference mode: {error}");
                error!("{message}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_error_progress(String::new(), message),
                )
                .await;
                if restart {
                    schedule_restart();
                }
                return;
            }
        };

        if mode != "local" {
            let readiness = async {
                let config = external_inference_config(&app_settings).await?;
                probe_external_inference_readiness(&http_client, &config).await
            }
            .await;
            let restart =
                finish_external_readiness_probe(&app, &initialization, generation, readiness).await;
            if restart {
                schedule_restart();
            }
            return;
        }

        let local_config = async {
            let model_id = match app_settings.get("generation_model").await? {
                Some(value) if !value.is_empty() => {
                    validate_setting("generation_model", &value)?;
                    value
                }
                _ => "unsloth/Qwen3-1.7B-GGUF".to_string(),
            };
            let tokenizer_model = app_settings
                .get("tokenizer_model")
                .await?
                .filter(|model| is_valid_hugging_face_model_id(model))
                .unwrap_or_else(|| DEFAULT_TOKENIZER_MODEL.to_string());
            let gguf_filename = app_settings
                .get("gguf_filename")
                .await?
                .filter(|value| is_valid_gguf_filename(value))
                .unwrap_or_else(|| "Qwen3-1.7B-Q4_K_M.gguf".to_string());
            Ok::<_, AppError>((model_id, tokenizer_model, gguf_filename))
        }
        .await;
        let (model_id, tokenizer_model, gguf_filename) = match local_config {
            Ok(config) => config,
            Err(error) => {
                let message = format!("Failed to read local model configuration: {error}");
                error!("{message}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_error_progress(String::new(), message),
                )
                .await;
                if restart {
                    schedule_restart();
                }
                return;
            }
        };

        if !llm_initialization_is_current(&initialization, generation).await {
            let restart = finish_llm_initialization(&initialization, generation).await;
            if restart {
                schedule_restart();
            }
            return;
        }

        info!("downloading model '{model_id}' ({gguf_filename}) via hf-hub...");
        let progress_initialization = initialization.clone();
        let progress_app = app.clone();
        let progress_emitter: ProgressEmitter = Arc::new(move |progress| {
            let Ok(mut current) = progress_initialization.try_lock() else {
                return;
            };
            if current.generation == generation
                && current.in_progress_generation == Some(generation)
            {
                publish_model_download_progress(&progress_app, &mut current, progress);
            }
        });
        let gguf_path = match model_download::download_model(
            &model_id,
            &gguf_filename,
            &models_dir,
            progress_emitter,
            cancel_rx,
        )
        .await
        {
            Ok(path) => path,
            Err(error) => {
                if !llm_initialization_is_current(&initialization, generation).await {
                    return;
                }
                error!("model download failed after all retries: {error}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_error_progress(model_id.to_string(), error.to_string()),
                )
                .await;
                if restart {
                    schedule_restart();
                }
                return;
            }
        };

        if !emit_model_download_progress_if_current(
            &app,
            &initialization,
            generation,
            ModelDownloadProgress {
                model_id: model_id.to_string(),
                current_file: String::new(),
                file_downloaded: 0,
                file_total: 0,
                files_completed: 1,
                total_files: 1,
                stage: DownloadStage::LoadingModel,
            },
        )
        .await
        {
            let restart = finish_llm_initialization(&initialization, generation).await;
            if restart {
                schedule_restart();
            }
            return;
        }

        let Some(parent_dir) = gguf_path.parent() else {
            let message = format!(
                "GGUF path has no parent directory: '{}'",
                gguf_path.display()
            );
            error!("{message}");
            let restart = finish_llm_initialization_with_progress(
                &app,
                &initialization,
                generation,
                model_error_progress(model_id.to_string(), message),
            )
            .await;
            if restart {
                schedule_restart();
            }
            return;
        };

        info!("model downloaded, building mistral.rs pipeline...");
        let builder = configure_local_generation(
            mistralrs::GgufModelBuilder::new(
                parent_dir.to_string_lossy().to_string(),
                vec![gguf_filename],
            )
            .with_tok_model_id(tokenizer_model),
        );

        match builder.build().await {
            Ok(model) => {
                let current_mode = match app_settings.get("inference_mode").await {
                    Ok(mode) => mode.unwrap_or_else(|| "local".to_string()),
                    Err(error) => {
                        let message = format!("Failed to read inference mode: {error}");
                        error!("{message}");
                        let restart = finish_llm_initialization_with_progress(
                            &app,
                            &initialization,
                            generation,
                            model_error_progress(String::new(), message),
                        )
                        .await;
                        if restart {
                            schedule_restart();
                        }
                        return;
                    }
                };
                if current_mode != "local" {
                    let restart = finish_llm_initialization(&initialization, generation).await;
                    if restart {
                        schedule_restart();
                    }
                    return;
                }

                let restart = finish_local_model_initialization(
                    &app,
                    &llm_state,
                    &initialization,
                    generation,
                    model,
                )
                .await;
                if restart {
                    schedule_restart();
                }
            }
            Err(error) => {
                let message = format!("Failed to initialize local model: {error}");
                error!("{message}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_error_progress(model_id, message),
                )
                .await;
                if restart {
                    schedule_restart();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gguf_filename_predicate_preserves_validation_and_fallback_policies() {
        let fallback = "Qwen3-1.7B-Q4_K_M.gguf";
        for value in ["model.gguf", ".gguf", " model.gguf", "模型.gguf"] {
            assert!(is_valid_gguf_filename(value));
            assert!(validate_setting("gguf_filename", value).is_ok());
            assert_eq!(
                Some(value)
                    .filter(|value| is_valid_gguf_filename(value))
                    .unwrap_or(fallback),
                value
            );
        }
        for value in [
            "",
            "model.GGUF",
            "model.gguf ",
            "dir/model.gguf",
            "dir\\model.gguf",
        ] {
            assert!(!is_valid_gguf_filename(value));
            let error = validate_setting("gguf_filename", value).unwrap_err();
            assert_eq!(error.code, domain::ErrorCode::InvalidInput);
            assert_eq!(error.operation, "Validate setting");
            assert_eq!(
                error.detail,
                "Unsupported or invalid setting: gguf_filename"
            );
            assert_eq!(
                Some(value)
                    .filter(|value| is_valid_gguf_filename(value))
                    .unwrap_or(fallback),
                fallback
            );
        }
    }

    #[test]
    fn model_error_progress_preserves_zeroed_wire_payload() {
        for model_id in ["", "unsloth/Qwen3-1.7B-GGUF"] {
            let progress = model_error_progress(model_id.to_string(), "failure detail".to_string());
            assert_eq!(
                serde_json::to_value(progress).unwrap(),
                serde_json::json!({
                    "model_id": model_id,
                    "current_file": "",
                    "file_downloaded": 0,
                    "file_total": 0,
                    "files_completed": 0,
                    "total_files": 0,
                    "type": "error",
                    "message": "failure detail",
                })
            );
        }
    }

    #[test]
    fn owned_finish_cleans_current_and_stale_tasks_without_clearing_readiness_or_progress() {
        for generation in [7, 8] {
            for restart_requested in [false, true] {
                let (sender, receiver) = tokio::sync::watch::channel(false);
                let progress = model_error_progress("model".to_string(), "prior".to_string());
                let mut current = LlmInitializationState {
                    generation,
                    in_progress_generation: Some(7),
                    restart_requested,
                    download_cancellation: Some(sender),
                    last_download_progress: Some(progress.clone()),
                    external_ready: true,
                    inference_mode: "external".to_string(),
                };

                assert_eq!(
                    finish_owned_llm_initialization(&mut current, 7),
                    Some(restart_requested)
                );
                assert_eq!(current.generation, generation);
                assert!(!current.is_in_progress());
                assert_eq!(current.in_progress_generation, None);
                assert!(!current.restart_requested);
                assert!(current.download_cancellation.is_none());
                assert!(!*receiver.borrow());
                assert!(receiver.has_changed().is_err());
                assert_eq!(current.last_download_progress, Some(progress));
                assert!(current.external_ready);
                assert_eq!(current.inference_mode, "external");
                assert_eq!(finish_owned_llm_initialization(&mut current, 7), None);
            }
        }
    }

    #[test]
    fn unowned_finish_leaves_active_and_idle_state_untouched() {
        for owner in [None, Some(8)] {
            let (sender, receiver) = tokio::sync::watch::channel(false);
            let progress = model_error_progress("model".to_string(), "prior".to_string());
            let mut current = LlmInitializationState {
                generation: 8,
                in_progress_generation: owner,
                restart_requested: true,
                download_cancellation: Some(sender),
                last_download_progress: Some(progress.clone()),
                external_ready: true,
                inference_mode: "external".to_string(),
            };

            assert_eq!(finish_owned_llm_initialization(&mut current, 7), None);
            assert_eq!(current.generation, 8);
            assert_eq!(current.is_in_progress(), owner.is_some());
            assert_eq!(current.in_progress_generation, owner);
            assert!(current.restart_requested);
            assert!(current.download_cancellation.is_some());
            assert!(!*receiver.borrow());
            assert!(!receiver.has_changed().unwrap());
            assert_eq!(current.last_download_progress, Some(progress));
            assert!(current.external_ready);
            assert_eq!(current.inference_mode, "external");
        }
    }

    #[test]
    fn validates_theme() {
        assert!(validate_setting("theme", "dark").is_ok());
        assert!(validate_setting("theme", "light").is_ok());
        assert!(validate_setting("theme", "system").is_err());
        assert!(validate_setting("theme", "").is_err());
    }

    #[test]
    fn validates_external_model() {
        assert!(validate_setting("external_model", "gpt-4o-mini").is_ok());
        assert!(validate_setting("external_model", "").is_err());
        assert!(validate_setting("external_model", "   ").is_err());
    }

    #[test]
    fn validates_tokenizer_model() {
        assert!(validate_setting("tokenizer_model", "Qwen/Qwen3-1.7B").is_ok());
        for invalid in [
            "",
            "Qwen",
            "Qwen/",
            "/Qwen3-1.7B",
            "Qwen/Qwen3/1.7B",
            "Qwen /Qwen3-1.7B",
        ] {
            assert!(validate_setting("tokenizer_model", invalid).is_err());
        }
    }

    #[test]
    fn rejects_generation_model_ids_without_both_components() {
        assert!(validate_setting("generation_model", "unsloth/Qwen3-1.7B-GGUF").is_ok());
        for malformed in [
            "",
            "unsloth",
            "unsloth/",
            "/Qwen3-1.7B-GGUF",
            "org/model/extra",
        ] {
            assert!(validate_setting("generation_model", malformed).is_err());
        }
    }

    #[tokio::test]
    async fn local_model_config_invalidations_coalesce_to_the_latest_generation() {
        let initialization = Arc::new(tokio::sync::Mutex::new(LlmInitializationState::default()));
        let first_generation = {
            let mut current = initialization.lock().await;
            let generation = current.generation;
            current.in_progress_generation = Some(generation);
            generation
        };

        {
            let mut current = initialization.lock().await;
            invalidate_llm_initialization(&mut current);
            current.restart_requested = true;
            invalidate_llm_initialization(&mut current);
            current.restart_requested = true;
            assert_eq!(current.generation, first_generation.wrapping_add(2));
            assert_eq!(current.in_progress_generation, Some(first_generation));
        }

        assert!(!llm_initialization_is_current(&initialization, first_generation).await);
        assert!(finish_llm_initialization(&initialization, first_generation).await);
        assert!(!finish_llm_initialization(&initialization, first_generation).await);

        let latest_generation = {
            let mut current = initialization.lock().await;
            assert!(!current.is_in_progress());
            assert_eq!(current.in_progress_generation, None);
            let generation = current.generation;
            current.in_progress_generation = Some(generation);
            generation
        };
        assert!(llm_initialization_is_current(&initialization, latest_generation).await);
        assert!(!finish_llm_initialization(&initialization, latest_generation).await);
    }

    #[tokio::test]
    async fn cancel_active_model_download_signals_receiver_and_resets_state() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let mut state = LlmInitializationState {
            generation: 1,
            in_progress_generation: Some(1),
            restart_requested: true,
            download_cancellation: Some(tx),
            last_download_progress: None,
            external_ready: false,
            inference_mode: "local".to_string(),
        };

        invalidate_llm_initialization(&mut state);
        if let Some(cancel) = state.download_cancellation.take() {
            let _ = cancel.send(true);
        }
        state.in_progress_generation = None;
        state.restart_requested = false;

        assert!(*rx.borrow());
        assert_eq!(state.generation, 2);
        assert!(!state.is_in_progress());
        assert_eq!(state.in_progress_generation, None);
        assert!(!state.restart_requested);
        assert!(state.download_cancellation.is_none());
    }
}
