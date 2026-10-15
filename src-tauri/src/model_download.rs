use domain::AppError;
use hf_hub::progress::{DownloadEvent, ProgressEvent, ProgressHandler};
use log::warn;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub use domain::{DownloadStage, ModelDownloadProgress};

pub type ProgressEmitter = Arc<dyn Fn(ModelDownloadProgress) + Send + Sync>;

fn download_cancelled_error() -> AppError {
    AppError::internal(
        "Download model file",
        "Model download was cancelled by the user",
        "Retry local model initialization when you are ready to download.",
    )
}

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
        return Err(download_cancelled_error());
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

    let cached =
        cached_model_path(client.clone(), model_id.to_string(), filename.to_string()).await?;
    let path = if let Some(path) = cached {
        path
    } else {
        loop {
            if *cancellation.borrow() {
                return Err(download_cancelled_error());
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
                        return Err(download_cancelled_error());
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
        }
    };

    let verification_path = path.clone();
    let verification_cache = cache_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        verify_cached_model(&verification_cache, &verification_path, &cancellation)
    })
    .await
    .map_err(|_| {
        AppError::internal(
            "Verify model file",
            "The model verification worker did not complete",
            "Restart the application and retry local model initialization.",
        )
    })??;
    let bytes = tokio::fs::metadata(&path).await?.len();
    (emit_progress.as_ref())(ModelDownloadProgress {
        model_id: model_id.to_string(),
        current_file: filename.to_string(),
        file_downloaded: bytes,
        file_total: bytes,
        files_completed: 1,
        total_files: 1,
        stage: DownloadStage::Complete,
    });
    Ok(path)
}

async fn cached_model_path(
    client: hf_hub::HFClient,
    model_id: String,
    filename: String,
) -> Result<Option<PathBuf>, AppError> {
    let runtime = tokio::runtime::Handle::current();
    // HF Hub's cache-only resolver performs synchronous filesystem work. Keep
    // it off the async executor and never probe a server on a cache hit.
    tokio::task::spawn_blocking(move || {
        runtime.block_on(async move {
            let (owner, name) = hf_hub::split_id(&model_id);
            client
                .model(owner, name)
                .download_file()
                .filename(filename)
                .local_files_only(true)
                .send()
                .await
                .ok()
        })
    })
    .await
    .map_err(|_| {
        AppError::internal(
            "Resolve cached model",
            "The model cache resolver did not complete",
            "Restart the application and retry local model initialization.",
        )
    })
}

// HF Hub's LFS cache stores GGUF blobs under their SHA-256 object ID. Verify
// fresh downloads and cache hits alike, without adding an online startup probe.
// This checks corruption against Hub metadata, not trust in a model publisher.
fn verify_cached_model(
    cache_dir: &std::path::Path,
    path: &std::path::Path,
    cancellation: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), AppError> {
    let cache = std::fs::canonicalize(cache_dir)?;
    let blob = std::fs::canonicalize(path)?;
    if !blob.starts_with(&cache) {
        return Err(model_integrity_error(
            "The cached model points outside its managed cache",
        ));
    }
    let expected = blob
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| model_integrity_error("The cached model has no SHA-256 content identity"))?;
    let mut file = std::fs::File::open(&blob)?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header)
        .map_err(|_| model_integrity_error("The cached model has an incomplete GGUF header"))?;
    if &header[..4] != b"GGUF"
        || !matches!(
            u32::from_le_bytes(header[4..].try_into().expect("fixed header slice")),
            2 | 3
        )
    {
        return Err(model_integrity_error(
            "The cached model is not a supported GGUF file",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(header);
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if *cancellation.borrow() || cancellation.has_changed().is_err() {
            return Err(AppError::internal(
                "Verify model file",
                "Model verification was cancelled",
                "Retry local model initialization when you are ready.",
            ));
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected.to_ascii_lowercase() {
        return Err(model_integrity_error(
            "The cached model does not match its SHA-256 content identity",
        ));
    }
    Ok(())
}

fn model_integrity_error(detail: &str) -> AppError {
    AppError::inference(
        "Verify model file",
        detail,
        "Remove the affected app-managed model cache and retry the download. Use a Hugging Face repository that provides a valid GGUF artifact.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_cancellation_error_preserves_its_payload() {
        let error = download_cancelled_error();
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "internal",
                "operation": "Download model file",
                "detail": "Model download was cancelled by the user",
                "recovery": "Retry local model initialization when you are ready to download.",
                "message": "Download model file: Model download was cancelled by the user. Retry local model initialization when you are ready to download.",
            })
        );
    }

    #[tokio::test]
    async fn cached_models_resolve_with_an_unreachable_hub_endpoint() {
        let dir =
            std::env::temp_dir().join(format!("ragtag-offline-cache-{}", uuid::Uuid::now_v7()));
        let revision = "a".repeat(40);
        let repo = dir.join("models--test--model");
        tokio::fs::create_dir_all(repo.join("refs")).await.unwrap();
        let snapshot = repo.join("snapshots").join(&revision);
        tokio::fs::create_dir_all(&snapshot).await.unwrap();
        tokio::fs::write(repo.join("refs/main"), &revision)
            .await
            .unwrap();
        tokio::fs::write(snapshot.join("model.gguf"), b"cached fixture")
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = hf_hub::HFClient::builder()
            .cache_dir(&dir)
            .token("")
            .endpoint(format!("http://{}", listener.local_addr().unwrap()))
            .build()
            .unwrap();
        let found = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cached_model_path(client.clone(), "test/model".into(), "model.gguf".into()),
        )
        .await
        .expect("cache lookup must not wait for an HTTP response")
        .unwrap();
        assert_eq!(found, Some(snapshot.join("model.gguf")));
        assert!(
            cached_model_path(client, "test/model".into(), "missing.gguf".into())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
                .await
                .is_err(),
            "cache-only resolution must not open a network connection"
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn verifies_content_addressed_models_and_rejects_corruption_without_network() {
        let dir =
            std::env::temp_dir().join(format!("ragtag-model-integrity-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = b"GGUF\x03\x00\x00\x00synthetic test payload";
        let name: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let (sender, receiver) = tokio::sync::watch::channel(false);
        verify_cached_model(&dir, &path, &receiver).unwrap();
        std::fs::write(&path, b"GGUF\x03\x00\x00\x00modified payload").unwrap();
        let error = verify_cached_model(&dir, &path, &receiver).unwrap_err();
        assert_eq!(error.code, domain::ErrorCode::Inference);
        assert!(error.detail.contains("SHA-256"));
        std::fs::write(&path, b"not a GGUF model").unwrap();
        assert!(
            verify_cached_model(&dir, &path, &receiver)
                .unwrap_err()
                .detail
                .contains("GGUF")
        );
        std::fs::write(&path, bytes).unwrap();
        sender.send(true).unwrap();
        assert!(
            verify_cached_model(&dir, &path, &receiver)
                .unwrap_err()
                .detail
                .contains("cancelled")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_cache_symlinks_outside_managed_storage_and_missing_identity() {
        let dir = std::env::temp_dir().join(format!("ragtag-model-cache-{}", uuid::Uuid::now_v7()));
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let outside = dir.join("outside");
        std::fs::write(&outside, b"GGUF\x03\x00\x00\x00test").unwrap();
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let error = verify_cached_model(&cache, &outside, &receiver).unwrap_err();
        assert!(error.detail.contains("outside"));
        #[cfg(unix)]
        {
            let link = cache.join("a".repeat(64));
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(
                verify_cached_model(&cache, &link, &receiver)
                    .unwrap_err()
                    .detail
                    .contains("outside")
            );
        }
        let unaddressed = cache.join("model.gguf");
        std::fs::write(&unaddressed, b"GGUF\x03\x00\x00\x00test").unwrap();
        assert!(
            verify_cached_model(&cache, &unaddressed, &receiver)
                .unwrap_err()
                .detail
                .contains("identity")
        );
        drop(sender);
        std::fs::remove_dir_all(dir).unwrap();
    }

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
