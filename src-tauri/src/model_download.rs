use domain::AppError;
use hf_hub::progress::{DownloadEvent, ProgressEvent, ProgressHandler};
use log::warn;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub use domain::{DownloadStage, ModelDownloadProgress};

pub type ProgressEmitter = Arc<dyn Fn(ModelDownloadProgress) + Send + Sync>;

#[derive(Clone)]
struct ProgressReporter {
    emit_progress: ProgressEmitter,
    model_id: String,
    filename: String,
    total_bytes: Arc<AtomicU64>,
}

impl ProgressHandler for ProgressReporter {
    fn on_progress(&self, event: &ProgressEvent) {
        let progress = match event {
            ProgressEvent::Download(DownloadEvent::Start {
                total_files,
                total_bytes,
            }) => {
                self.total_bytes.store(*total_bytes, Ordering::Relaxed);
                Some(ModelDownloadProgress {
                    model_id: self.model_id.clone(),
                    current_file: self.filename.clone(),
                    file_downloaded: 0,
                    file_total: *total_bytes,
                    files_completed: 0,
                    total_files: *total_files,
                    stage: DownloadStage::Downloading,
                })
            }
            ProgressEvent::Download(DownloadEvent::Progress { files }) => files
                .iter()
                .find(|file| file.filename == self.filename)
                .or_else(|| files.first())
                .map(|file| {
                    self.total_bytes.store(file.total_bytes, Ordering::Relaxed);
                    ModelDownloadProgress {
                        model_id: self.model_id.clone(),
                        current_file: file.filename.clone(),
                        file_downloaded: file.bytes_completed,
                        file_total: file.total_bytes,
                        files_completed: 0,
                        total_files: 1,
                        stage: DownloadStage::Downloading,
                    }
                }),
            ProgressEvent::Download(DownloadEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            }) => {
                self.total_bytes.store(*total_bytes, Ordering::Relaxed);
                Some(ModelDownloadProgress {
                    model_id: self.model_id.clone(),
                    current_file: self.filename.clone(),
                    file_downloaded: *bytes_completed,
                    file_total: *total_bytes,
                    files_completed: 0,
                    total_files: 1,
                    stage: DownloadStage::Downloading,
                })
            }
            ProgressEvent::Download(DownloadEvent::Complete) => {
                let total_bytes = self.total_bytes.load(Ordering::Relaxed);
                Some(ModelDownloadProgress {
                    model_id: self.model_id.clone(),
                    current_file: self.filename.clone(),
                    file_downloaded: total_bytes,
                    file_total: total_bytes,
                    files_completed: 1,
                    total_files: 1,
                    stage: DownloadStage::Complete,
                })
            }
            _ => None,
        };

        if let Some(progress) = progress {
            (self.emit_progress.as_ref())(progress);
        }
    }
}

pub async fn download_model(
    model_id: &str,
    filename: &str,
    cache_dir: &std::path::Path,
    emit_progress: ProgressEmitter,
    mut cancellation: tokio::sync::watch::Receiver<bool>,
) -> Result<PathBuf, AppError> {
    if *cancellation.borrow() {
        return Err(AppError::internal(
            "Download model file",
            "Model download was cancelled by the user",
            "Retry local model initialization when you are ready to download.",
        ));
    }

    tokio::fs::create_dir_all(cache_dir).await?;

    let client = hf_hub::HFClient::builder()
        .cache_dir(cache_dir)
        .build()
        .map_err(|e| {
            AppError::internal(
                "Initialize model download client",
                format!("Failed to initialize Hugging Face Hub client: {e}"),
                "Verify network connectivity and cache directory permissions.",
            )
        })?;
    let (owner, name) = hf_hub::split_id(model_id);
    let repo = client.model(owner, name);

    (emit_progress.as_ref())(ModelDownloadProgress {
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
        if *cancellation.borrow() {
            return Err(AppError::internal(
                "Download model file",
                "Model download was cancelled by the user",
                "Retry local model initialization when you are ready to download.",
            ));
        }
        attempts += 1;

        let reporter = ProgressReporter {
            emit_progress: emit_progress.clone(),
            model_id: model_id.to_string(),
            filename: filename.to_string(),
            total_bytes: Arc::new(AtomicU64::new(0)),
        };

        let download_future = repo
            .download_file()
            .filename(filename)
            .progress(reporter)
            .send();

        let download_result = tokio::select! {
            biased;
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    return Err(AppError::internal(
                        "Download model file",
                        "Model download was cancelled by the user",
                        "Retry local model initialization when you are ready to download.",
                    ));
                }
                continue;
            }
            res = download_future => res,
        };

        match download_result {
            Ok(path) => break path,
            Err(e) if attempts < max_attempts => {
                warn!("download attempt {attempts}/{max_attempts} failed — retrying ({e})");
            }
            Err(e) => {
                return Err(AppError::inference(
                    "Download model file",
                    format!(
                        "Failed to download {filename} from {model_id} after {max_attempts} attempts: {e}"
                    ),
                    "Check network connectivity or verify the model repository and filename settings.",
                ));
            }
        }
    };

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_immediately_if_cancellation_already_active() {
        let (tx, rx) = tokio::sync::watch::channel(true);
        let emitter: ProgressEmitter = Arc::new(|_| {});
        let temp_dir = std::env::temp_dir().join(uuid::Uuid::now_v7().to_string());
        let res = download_model("test/model", "model.gguf", &temp_dir, emitter, rx).await;
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, domain::ErrorCode::Internal);
        assert!(err.detail.contains("cancelled by the user"));
        drop(tx);
    }

    #[tokio::test]
    async fn aborts_immediately_if_cancellation_sender_is_dropped() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        drop(tx);
        let emitter: ProgressEmitter = Arc::new(|_| {});
        let temp_dir = std::env::temp_dir().join(uuid::Uuid::now_v7().to_string());
        let res = download_model("test/model", "model.gguf", &temp_dir, emitter, rx).await;
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, domain::ErrorCode::Internal);
        assert!(err.detail.contains("cancelled by the user"));
    }
}

