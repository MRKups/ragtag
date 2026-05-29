use domain::AppError;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Debug)]
pub struct ModelDownloadProgress {
    pub model_id: String,
    pub current_file: String,
    pub file_downloaded: u64,
    pub file_total: u64,
    pub files_completed: usize,
    pub total_files: usize,
    pub stage: DownloadStage,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStage {
    FetchingInfo,
    Downloading,
    Complete,
    Error { message: String },
}

#[derive(Clone)]
struct ProgressReporter {
    app_handle: AppHandle,
    model_id: String,
    current_file: Arc<Mutex<String>>,
    files_completed: Arc<Mutex<usize>>,
    total_files: usize,
}

impl hf_hub::api::tokio::Progress for ProgressReporter {
    async fn init(&mut self, size: usize, filename: &str) {
        *self.current_file.lock().await = filename.to_string();
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: filename.to_string(),
            file_downloaded: 0,
            file_total: size as u64,
            files_completed: *self.files_completed.lock().await,
            total_files: self.total_files,
            stage: DownloadStage::Downloading,
        });
    }

    async fn update(&mut self, size: usize) {
        let current_file = self.current_file.lock().await.clone();
        // size here is the cumulative bytes downloaded for this file so far
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file,
            file_downloaded: size as u64,
            file_total: 0, // unknown at update time
            files_completed: *self.files_completed.lock().await,
            total_files: self.total_files,
            stage: DownloadStage::Downloading,
        });
    }

    async fn finish(&mut self) {
        let mut completed = self.files_completed.lock().await;
        *completed += 1;
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: String::new(),
            file_downloaded: 0,
            file_total: 0,
            files_completed: *completed,
            total_files: self.total_files,
            stage: DownloadStage::Downloading,
        });
    }
}

pub async fn download_model(
    model_id: &str,
    cache_dir: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<(), AppError> {
    std::fs::create_dir_all(cache_dir)?;

    let api = hf_hub::api::tokio::ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .build()
        .map_err(|e| AppError::Internal(format!("hf-hub api init: {e}")))?;

    let repo = api.model(model_id.to_string());

    emit_progress(app_handle, model_id, "", 0, 0, 0, 0, DownloadStage::FetchingInfo);

    let info = repo.info().await.map_err(|e| {
        AppError::Internal(format!("Failed to fetch model info for {model_id}: {e}"))
    })?;

    let siblings = info.siblings;
    if siblings.is_empty() {
        return Err(AppError::Internal(format!("No files found for model {model_id}")));
    }

    let total = siblings.len();
    let files_completed = Arc::new(Mutex::new(0usize));
    let current_file = Arc::new(Mutex::new(String::new()));

    for sibling in &siblings {
        let reporter = ProgressReporter {
            app_handle: app_handle.clone(),
            model_id: model_id.to_string(),
            current_file: current_file.clone(),
            files_completed: files_completed.clone(),
            total_files: total,
        };

        repo.download_with_progress(&sibling.rfilename, reporter)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to download {} from {model_id}: {e}",
                    sibling.rfilename
                ))
            })?;
    }

    emit_progress(app_handle, model_id, "", 0, 0, total, total, DownloadStage::Complete);

    Ok(())
}

fn emit_progress(
    app_handle: &AppHandle,
    model_id: &str,
    current_file: &str,
    file_downloaded: u64,
    file_total: u64,
    files_completed: usize,
    total_files: usize,
    stage: DownloadStage,
) {
    let _ = app_handle.emit("model_download_progress", ModelDownloadProgress {
        model_id: model_id.to_string(),
        current_file: current_file.to_string(),
        file_downloaded,
        file_total,
        files_completed,
        total_files,
        stage,
    });
}
