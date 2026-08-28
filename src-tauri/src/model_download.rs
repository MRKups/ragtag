use domain::AppError;
use hf_hub::progress::{DownloadEvent, ProgressEvent, ProgressHandler};
use log::warn;
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
    #[serde(flatten)]
    pub stage: DownloadStage,
}

#[derive(Clone, Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
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

impl ProgressHandler for ProgressReporter {
    fn on_progress(&self, event: &ProgressEvent) {
        let progress = match event {
            ProgressEvent::Download(DownloadEvent::Start {
                total_files,
                total_bytes,
            }) => Some(ModelDownloadProgress {
                model_id: self.model_id.clone(),
                current_file: self.filename.clone(),
                file_downloaded: 0,
                file_total: *total_bytes,
                files_completed: 0,
                total_files: *total_files,
                stage: DownloadStage::Downloading,
            }),
            ProgressEvent::Download(DownloadEvent::Progress { files }) => files
                .iter()
                .find(|file| file.filename == self.filename)
                .map(|file| ModelDownloadProgress {
                    model_id: self.model_id.clone(),
                    current_file: file.filename.clone(),
                    file_downloaded: file.bytes_completed,
                    file_total: file.total_bytes,
                    files_completed: 0,
                    total_files: 1,
                    stage: DownloadStage::Downloading,
                }),
            ProgressEvent::Download(DownloadEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            }) => Some(ModelDownloadProgress {
                model_id: self.model_id.clone(),
                current_file: self.filename.clone(),
                file_downloaded: *bytes_completed,
                file_total: *total_bytes,
                files_completed: 0,
                total_files: 1,
                stage: DownloadStage::Downloading,
            }),
            ProgressEvent::Download(DownloadEvent::Complete) => Some(ModelDownloadProgress {
                model_id: self.model_id.clone(),
                current_file: self.filename.clone(),
                file_downloaded: 0,
                file_total: 0,
                files_completed: 1,
                total_files: 1,
                stage: DownloadStage::Complete,
            }),
            _ => None,
        };

        if let Some(progress) = progress {
            let _ = self.app_handle.emit("model_download_progress", progress);
        }
    }
}

pub async fn download_model(
    model_id: &str,
    filename: &str,
    cache_dir: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(cache_dir).await?;

    let client = hf_hub::HFClient::builder()
        .cache_dir(cache_dir)
        .build()
        .map_err(|e| AppError::Internal(format!("hf-hub client init: {e}")))?;
    let (owner, name) = hf_hub::split_id(model_id);
    let repo = client.model(owner, name);

    let _ = app_handle.emit(
        "model_download_progress",
        ModelDownloadProgress {
            model_id: model_id.to_string(),
            current_file: String::new(),
            file_downloaded: 0,
            file_total: 0,
            files_completed: 0,
            total_files: 1,
            stage: DownloadStage::FetchingInfo,
        },
    );

    let mut attempts = 0u32;
    let max_attempts = 3u32;

    let path = loop {
        attempts += 1;

        let reporter = ProgressReporter {
            app_handle: app_handle.clone(),
            model_id: model_id.to_string(),
            filename: filename.to_string(),
        };

        match repo
            .download_file()
            .filename(filename)
            .progress(reporter)
            .send()
            .await
        {
            Ok(path) => break path,
            Err(e) if attempts < max_attempts => {
                warn!("download attempt {attempts}/{max_attempts} failed — retrying ({e})");
            }
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "Failed to download {filename} from {model_id} after {max_attempts} attempts: {e}"
                )));
            }
        }
    };

    Ok(path)
}
