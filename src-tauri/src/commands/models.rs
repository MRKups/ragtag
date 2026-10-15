use domain::AppError;
use log::error;
use persistence::AppSettingsRepository;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use crate::model_lifecycle::{recover_startup_documents_with_retry, request_llm_initialization};
use crate::state::AppState;

static EMBEDDER_INIT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn begin_first_run_init(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let mode = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    let embedder_ready = state.embedder.read().await.is_some();
    let llm_ready = mode == "local" && state.local_llm.read().await.is_some();

    if embedder_ready {
        let _ = app.emit("embedder_ready", ());
    }
    if llm_ready {
        let _ = app.emit("llm_ready", ());
    }

    if embedder_ready && llm_ready {
        return Ok(());
    }

    let models_dir =
        crate::paths::models_dir(&app, "Verify application local storage permissions.")?;

    if !embedder_ready && !EMBEDDER_INIT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        let embedder_state = state.embedder.clone();
        let embedder_handle = app.clone();
        let models_dir_embedder = models_dir.clone();
        let embedder_pool = state.db_pool.clone();
        let embedder_job_queue = state.job_queue.clone();
        let recovery_vector_db = state.vector_db.clone();
        let document_operations = state.document_operations.clone();

        tauri::async_runtime::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                rag_pipeline::init_embedder(&models_dir_embedder)
            })
            .await;

            match result {
                Ok(Ok(emb)) => {
                    *embedder_state.write().await = Some(Arc::new(std::sync::Mutex::new(emb)));
                    let _ = embedder_handle.emit("embedder_ready", ());

                    // Recovery begins only after the embedder is available. Database reads
                    // retry with bounded backoff so durable work is never deferred to restart.
                    tauri::async_runtime::spawn(recover_startup_documents_with_retry(
                        embedder_pool,
                        recovery_vector_db,
                        document_operations,
                        embedder_job_queue,
                    ));
                }
                Ok(Err(e)) => {
                    EMBEDDER_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
                    error!("embedder initialization failed: {e}");
                    let _ = embedder_handle.emit(
                        "embedder_failed",
                        serde_json::json!({
                            "message": e.to_string(),
                        }),
                    );
                }
                Err(e) => {
                    EMBEDDER_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
                    error!("embedder initialization task panicked: {e}");
                    let _ = embedder_handle.emit(
                        "embedder_failed",
                        serde_json::json!({
                            "message": format!("Embedder task panicked: {e}"),
                        }),
                    );
                }
            }
        });
    }

    if !llm_ready {
        request_llm_initialization(&app, &state, models_dir).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn retry_local_model_initialization(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let mode = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    if mode != "local" {
        return Err(AppError::invalid_input(
            "Retry local model initialization",
            "Local model retry is unavailable while external inference is selected",
            "Switch inference mode to local in settings before retrying local model initialization.",
        ));
    }
    if state.local_llm.read().await.is_some() {
        let _ = app.emit("llm_ready", ());
        return Ok(());
    }

    let models_dir =
        crate::paths::models_dir(&app, "Verify application local storage permissions.")?;
    request_llm_initialization(&app, &state, models_dir).await;
    Ok(())
}

#[tauri::command]
pub async fn cancel_model_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Protected)?;
    crate::model_lifecycle::cancel_active_model_download(&app, &state).await
}

#[tauri::command]
pub async fn get_app_state(
    state: tauri::State<'_, AppState>,
) -> Result<domain::AppLifecycleState, AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Protected)?;

    let embedder_ready = state.embedder.read().await.is_some();
    let is_local_model_ready = state.local_llm.read().await.is_some();

    let init = state.llm_initialization.lock().await;
    let llm_ready = is_local_model_ready || init.external_ready;
    let inference_mode = if init.inference_mode.is_empty() {
        "local".to_string()
    } else {
        init.inference_mode.clone()
    };
    let model_download_active = init.is_in_progress();
    let model_download_progress = init.last_download_progress.clone();
    drop(init);

    let active_queries_count = state.active_queries.lock().map(|q| q.len()).unwrap_or(0);

    let active_doc = state
        .active_document_id
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let ingestion_active = active_doc.is_some();

    let capacity = state.capacity.snapshot();

    Ok(domain::AppLifecycleState {
        embedder_ready,
        llm_ready,
        inference_mode,
        model_download_active,
        model_download_progress,
        active_queries_count,
        ingestion_active,
        active_document_id: active_doc,
        capacity,
    })
}

#[tauri::command]
pub fn get_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_current_platform() {
        let platform = get_platform();
        assert!(["macos", "windows", "linux"].contains(&platform));
        #[cfg(target_os = "macos")]
        assert_eq!(platform, "macos");
        #[cfg(target_os = "windows")]
        assert_eq!(platform, "windows");
        #[cfg(target_os = "linux")]
        assert_eq!(platform, "linux");
    }
}
