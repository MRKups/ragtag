use domain::AppError;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use log::warn;

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
    file_total: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl hf_hub::api::tokio::Progress for ProgressReporter {
    async fn init(&mut self, size: usize, _filename: &str) {
        let total = size as u64;
        self.file_total.store(total, std::sync::atomic::Ordering::Relaxed);
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: self.filename.clone(),
            file_downloaded: 0,
            file_total: total,
            files_completed: 0,
            total_files: 1,
            stage: DownloadStage::Downloading,
        });
    }

    async fn update(&mut self, size: usize) {
        let total = self.file_total.load(std::sync::atomic::Ordering::Relaxed);
        let _ = self.app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: self.model_id.clone(),
            current_file: self.filename.clone(),
            file_downloaded: size as u64,
            file_total: total,
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

fn clean_stale_cache_files(cache_dir: &std::path::Path, model_id: &str) {
    // hf-hub folder naming: models--org--repo_name
    let repo_folder = format!("models--{}", model_id.replace('/', "--"));
    let blobs_dir = cache_dir.join(&repo_folder).join("blobs");
    if let Ok(entries) = std::fs::read_dir(&blobs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if matches!(ext, Some("sync.part" | "part" | "lock")) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

pub async fn download_model(
    model_id: &str,
    filename: &str,
    cache_dir: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<PathBuf, AppError> {
    std::fs::create_dir_all(cache_dir)?;

    clean_stale_cache_files(cache_dir, model_id);

    // Fast path: check if the file is already cached
    let cache = hf_hub::Cache::new(cache_dir.to_path_buf());
    let repo_cache = cache.repo(hf_hub::Repo::model(model_id.to_string()));
    if let Some(path) = repo_cache.get(filename) {
        let _ = app_handle.emit("model_download_progress", ModelDownloadProgress {
            model_id: model_id.to_string(),
            current_file: filename.to_string(),
            file_downloaded: 0,
            file_total: 0,
            files_completed: 1,
            total_files: 1,
            stage: DownloadStage::Complete,
        });
        return Ok(path);
    }

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

    let mut attempts = 0u32;
    let max_attempts = 3u32;

    let path = loop {
        attempts += 1;

        let reporter = ProgressReporter {
            app_handle: app_handle.clone(),
            model_id: model_id.to_string(),
            filename: filename.to_string(),
            file_total: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };

        match repo.download_with_progress(filename, reporter).await {
            Ok(p) => break p,
            Err(e) if attempts < max_attempts => {
                warn!("download attempt {attempts}/{max_attempts} failed — cleaning stale files and retrying ({e})");
                clean_stale_cache_files(cache_dir, model_id);
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
