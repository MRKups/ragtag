use domain::AppError;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

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
    filename: String,
}

impl hf_hub::api::tokio::Progress for ProgressReporter {
    async fn init(&mut self, size: usize, _filename: &str) {
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: self.filename.clone(),
            file_downloaded: 0,
            file_total: size as u64,
            files_completed: 0,
            total_files: 1,
            stage: DownloadStage::Downloading,
        });
    }

    async fn update(&mut self, size: usize) {
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: self.filename.clone(),
            file_downloaded: size as u64,
            file_total: 0,
            files_completed: 0,
            total_files: 1,
            stage: DownloadStage::Downloading,
        });
    }

    async fn finish(&mut self) {
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: String::new(),
            file_downloaded: 0,
            file_total: 0,
            files_completed: 1,
            total_files: 1,
            stage: DownloadStage::Complete,
        });
    }
}

pub async fn download_model(
    model_id: &str,
    filename: &str,
    cache_dir: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<PathBuf, AppError> {
    std::fs::create_dir_all(cache_dir)?;

    let api = hf_hub::api::tokio::ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .build()
        .map_err(|e| AppError::Internal(format!("hf-hub api init: {e}")))?;

    let repo = api.model(model_id.to_string());

    let _ = app_handle.emit("model_download_progress", ModelDownloadProgress {
        model_id: model_id.to_string(),
        current_file: String::new(),
        file_downloaded: 0,
        file_total: 0,
        files_completed: 0,
        total_files: 1,
        stage: DownloadStage::FetchingInfo,
    });

    let reporter = ProgressReporter {
        app_handle: app_handle.clone(),
        model_id: model_id.to_string(),
        filename: filename.to_string(),
    };

    repo.download_with_progress(filename, reporter)
        .await
        .map_err(|e| {
            AppError::Internal(format!(
                "Failed to download {filename} from {model_id}: {e}"
            ))
        })
}
