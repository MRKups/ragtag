mod state;
pub use state::AppState;
mod model_download;

use domain::{AppError, Citation, Document, DocumentStatus, TokenEvent};
use persistence::{init_pool, DocumentRepository, AppSettingsRepository, VectorDb};
use sha2::{Digest, Sha512};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use log::{debug, error, info, warn};

static EMBEDDER_INIT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LLM_INIT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
const DEFAULT_EXTERNAL_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TOKENIZER_MODEL: &str = "Qwen/Qwen3-1.7B";



const EXTRACTION_VERSION: &str = "page-aware-v1";

fn validate_upload(filename: &str, bytes: &[u8]) -> Result<(), AppError> {
    let name = std::path::Path::new(filename);
    if filename.is_empty()
        || filename.len() > 255
        || name.file_name().and_then(|name| name.to_str()) != Some(filename)
        || !filename.to_ascii_lowercase().ends_with(".pdf")
    {
        return Err(AppError::InvalidInput("Upload filename must be a PDF filename".into()));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::InvalidInput("PDF exceeds the 50 MiB upload limit".into()));
    }
    if !bytes.get(..1024).is_some_and(|prefix| prefix.windows(5).any(|window| window == b"%PDF-")) {
        return Err(AppError::InvalidInput("Upload is not a PDF".into()));
    }
    Ok(())
}

fn is_valid_hugging_face_model_id(value: &str) -> bool {
    value.split_once('/').is_some_and(|(organization, model)| {
        !organization.is_empty()
            && !model.is_empty()
            && !model.contains('/')
            && !value.contains(char::is_whitespace)
    })
}

fn validate_setting(key: &str, value: &str) -> Result<(), AppError> {
    match key {
        "inference_mode" if matches!(value, "local" | "external") => Ok(()),
        "use_query_enhancement" if matches!(value, "true" | "false") => Ok(()),
        "chunk_size" => value.parse::<usize>().ok().filter(|size| (128..=2048).contains(size))
            .map(|_| ()).ok_or_else(|| AppError::InvalidInput("chunk_size must be 128–2048".into())),
        "chunk_overlap" => value.parse::<usize>().ok().filter(|size| *size <= 1024)
            .map(|_| ()).ok_or_else(|| AppError::InvalidInput("chunk_overlap must be 0–1024".into())),
        "external_base_url" if reqwest::Url::parse(value).ok().is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some() && url.username().is_empty() && url.password().is_none()) => Ok(()),
        "external_model" if !value.trim().is_empty() => Ok(()),

        "generation_model" if value.split('/').count() == 2 && !value.contains(char::is_whitespace) => Ok(()),
        "tokenizer_model" if is_valid_hugging_face_model_id(value) => Ok(()),
        "gguf_filename" if !value.is_empty() && !value.contains('/') && value.ends_with(".gguf") => Ok(()),
        _ => Err(AppError::InvalidInput(format!("Unsupported or invalid setting: {key}"))),
    }
}

struct ActiveQuery {
    queries: Arc<std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    request_id: String,
}

impl Drop for ActiveQuery {
    fn drop(&mut self) {
        if let Ok(mut queries) = self.queries.lock() {
            queries.remove(&self.request_id);
        }
    }
}

#[tauri::command]
async fn begin_first_run_init(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let embedder_ready = state.embedder.read().await.is_some();
    let llm_ready = state.local_llm.read().await.is_some();

    if embedder_ready {
        let _ = app.emit("embedder_ready", ());
    }
    if llm_ready {
        let _ = app.emit("llm_ready", ());
    }

    if embedder_ready && llm_ready {
        return Ok(());
    }

    let models_dir = app.path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("models");

    if !embedder_ready && !EMBEDDER_INIT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
    let embedder_state = state.embedder.clone();
    let embedder_handle = app.clone();
    let models_dir_embedder = models_dir.clone();
    let embedder_pool = state.db_pool.clone();
    let embedder_job_queue = state.job_queue.clone();

    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            rag_pipeline::init_embedder(&models_dir_embedder)
        }).await;

        match result {
            Ok(Ok(emb)) => {
                *embedder_state.write().await = Some(Arc::new(std::sync::Mutex::new(emb)));
                let _ = embedder_handle.emit("embedder_ready", ());

                // Recovery runs after the embedder is ready. Awaiting sends applies
                // producer backpressure instead of silently dropping recovered work.
                let repo = persistence::DocumentRepository::new(embedder_pool.clone());

                let mut recovered_ids = HashSet::new();
                let settings = AppSettingsRepository::new(embedder_pool.clone());
                match settings.get("extraction_version").await {
                    Ok(Some(version)) if version == EXTRACTION_VERSION => {}
                    Ok(_) => match repo.requeue_indexed_for_reindex().await {
                        Ok(ids) => {
                            recovered_ids.extend(ids);
                            if let Err(e) = settings.set("extraction_version", EXTRACTION_VERSION).await {
                                error!("citation reindex — failed to record extraction version: {e}");
                            }
                        }
                        Err(e) => error!("citation reindex — failed to requeue indexed documents: {e}"),
                    },
                    Err(e) => error!("citation reindex — failed to read extraction version: {e}"),
                }

                match repo.reset_embedding_to_queued().await {
                    Ok(ids) => recovered_ids.extend(ids),
                    Err(e) => error!("startup recovery — failed to reset embedding documents: {e}"),
                }
                match repo.reset_failed_to_queued(3).await {
                    Ok(ids) => recovered_ids.extend(ids),
                    Err(e) => error!("startup recovery — failed to reset retryable documents: {e}"),
                }
                match repo.get_queued_ids().await {
                    Ok(ids) => recovered_ids.extend(ids),
                    Err(e) => error!("startup recovery — failed to list queued documents: {e}"),
                }

                let recovered_count = recovered_ids.len();
                for id in recovered_ids {
                    if let Err(e) = embedder_job_queue.send(id).await {
                        error!("startup recovery — document worker stopped before recovery completed: {e}");
                        break;
                    }
                }
                if recovered_count > 0 {
                    info!("startup recovery — re-queued {recovered_count} documents");
                } else {
                    debug!("startup recovery — no documents to resume");
                }
            }
            Ok(Err(e)) => {
                EMBEDDER_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
                error!("embedder initialization failed: {e}");
                let _ = embedder_handle.emit("embedder_failed", serde_json::json!({
                    "message": e.to_string(),
                }));
            }
            Err(e) => {
                EMBEDDER_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
                error!("embedder initialization task panicked: {e}");
                let _ = embedder_handle.emit("embedder_failed", serde_json::json!({
                    "message": format!("Embedder task panicked: {e}"),
                }));
            }
        }
    });
    }

    if !llm_ready && !LLM_INIT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
    let llm_state = state.local_llm.clone();
    let llm_handle = app.clone();
    let pool = state.db_pool.clone();
    let models_dir_llm = models_dir;

    tauri::async_runtime::spawn(async move {
        let app_settings = persistence::AppSettingsRepository::new(pool);
        let mode = app_settings.get("inference_mode").await
            .unwrap_or_default()
            .unwrap_or_else(|| "local".to_string());

        if mode != "local" {
            info!("BYO inference mode — skipping local LLM initialization");
            let _ = llm_handle.emit("llm_ready", ());
            return;
        }

        let model_id = app_settings.get("generation_model").await
            .unwrap_or_default()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unsloth/Qwen3-1.7B-GGUF".to_string());

        let tokenizer_model = app_settings.get("tokenizer_model").await
            .unwrap_or_default()
            .filter(|model| is_valid_hugging_face_model_id(model))
            .unwrap_or_else(|| DEFAULT_TOKENIZER_MODEL.to_string());

        let gguf_filename = app_settings.get("gguf_filename").await
            .unwrap_or_default()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Qwen3-1.7B-Q4_K_M.gguf".to_string());

        info!("downloading model '{model_id}' ({gguf_filename}) via hf-hub...");

        let gguf_path = match model_download::download_model(
            &model_id,
            &gguf_filename,
            &models_dir_llm,
            &llm_handle,
        ).await {
            Ok(p) => p,
            Err(e) => {
                error!("model download failed after all retries: {e}");
                let _ = llm_handle.emit("model_download_progress", model_download::ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    current_file: String::new(),
                    file_downloaded: 0,
                    file_total: 0,
                    files_completed: 0,
                    total_files: 0,
                    stage: model_download::DownloadStage::Error { message: e.to_string() },
                });
                LLM_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
                return;
            }
        };

        let _ = llm_handle.emit(
            "model_download_progress",
            model_download::ModelDownloadProgress {
                model_id: model_id.to_string(),
                current_file: String::new(),
                file_downloaded: 0,
                file_total: 0,
                files_completed: 1,
                total_files: 1,
                stage: model_download::DownloadStage::LoadingModel,
            },
        );

        info!("model downloaded, building mistral.rs pipeline...");

        let Some(parent_dir) = gguf_path.parent() else {
            let msg = format!(
                "GGUF path has no parent directory: '{}'",
                gguf_path.display()
            );
            error!("{msg}");
            let _ = llm_handle.emit("model_download_progress", model_download::ModelDownloadProgress {
                model_id: model_id.to_string(),
                current_file: String::new(),
                file_downloaded: 0,
                file_total: 0,
                files_completed: 0,
                total_files: 0,
                stage: model_download::DownloadStage::Error { message: msg },
            });
            LLM_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
            return;
        };

        let builder = mistralrs::GgufModelBuilder::new(
            parent_dir.to_string_lossy().to_string(),
            vec![gguf_filename.to_string()],
        )
        .with_tok_model_id(tokenizer_model);

        match builder.build().await {
            Ok(model) => {
                *llm_state.write().await = Some(Arc::new(model));
                info!("mistral.rs pipeline built and ready");
                let _ = llm_handle.emit("llm_ready", ());
            }
            Err(e) => {
                let msg = format!("Failed to initialize local model: {e}");
                error!("{msg}");
                let _ = llm_handle.emit("model_download_progress", model_download::ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    current_file: String::new(),
                    file_downloaded: 0,
                    file_total: 0,
                    files_completed: 0,
                    total_files: 0,
                    stage: model_download::DownloadStage::Error { message: msg },
                });
                LLM_INIT_IN_PROGRESS.store(false, Ordering::SeqCst);
            }
        }
    });
    }

    Ok(())
}

#[tauri::command]
async fn upload_document(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<String, AppError> {
    let filename = request.headers()
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Io("Missing filename header".into()))?
        .to_string();
    let file_bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => b.clone(),
        tauri::ipc::InvokeBody::Json(_) => return Err(AppError::Io("Expected raw binary body".into())),
    };

    validate_upload(&filename, &file_bytes)?;
    let state = app.state::<AppState>();
    let repo = DocumentRepository::new(state.db_pool.clone());

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let docs_dir = app_local_data.join("documents");
    tokio::fs::create_dir_all(&docs_dir).await?;

    let hash = tokio::task::spawn_blocking({
        let bytes = file_bytes.clone();
        move || {
            let mut hasher = Sha512::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("Hashing failed: {}", e)))?;

    if let Some(existing) = repo.find_by_hash(&hash).await? {
        return Err(AppError::DuplicateDocument(
            format!("This file was already uploaded as '{}' (status: {:?})", existing.filename, existing.status)
        ));
    }

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.pdf", &doc_id);
    let storage_path = docs_dir.join(&storage_filename);
    let relative_storage_path = format!("documents/{}", storage_filename);

    tokio::fs::write(&storage_path, &file_bytes).await?;

    let created_at = jiff::Zoned::now()
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string();

    let doc = Document {
        id: doc_id.clone(),
        filename,
        file_type: "pdf".to_string(),
        file_size_bytes: Some(file_bytes.len() as i64),
        file_hash: Some(hash),
        storage_path: relative_storage_path,
        status: DocumentStatus::Queued,
        chunk_count: None,
        embedding_model: None,
        embedding_attempts: 0,
        created_at: Some(created_at),
        processing_started_at: None,
        indexed_at: None,
        error_message: None,
        retry_count: 0,
    };

    if let Err(e) = repo.insert(&doc).await {
        let _ = tokio::fs::remove_file(&storage_path).await;
        return Err(e);
    }

    if state.job_queue.try_send(doc_id.clone()).is_err() {
        repo.delete_by_id(&doc_id).await?;
        let _ = tokio::fs::remove_file(&storage_path).await;
        return Err(AppError::QueueFull);
    }

    Ok(doc_id)
}

#[tauri::command]
async fn execute_query(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    query: String,
    request_id: String,
    channel: tauri::ipc::Channel<TokenEvent>,
) -> Result<(), AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    let (cancel_sender, mut cancel_receiver) = tokio::sync::watch::channel(false);
    state
        .active_queries
        .lock()
        .map_err(|_| AppError::Internal("Query cancellation state is poisoned".into()))?
        .insert(request_id.clone(), cancel_sender);
    let _active_query = ActiveQuery {
        queries: state.active_queries.clone(),
        request_id: request_id.clone(),
    };
    let embedder_opt = {
        let guard = state.embedder.read().await;
        guard.clone()
    };
    let embedder = embedder_opt
        .ok_or_else(|| AppError::Internal("Embedding model not yet loaded. Please wait.".into()))?;
    let vector_db = state.vector_db.clone();
    let local_llm = state.local_llm.clone();
    let http_client = state.http_client.clone();
    let settings_repo = AppSettingsRepository::new(state.db_pool.clone());

    let use_query_enhancement = settings_repo
        .get("use_query_enhancement")
        .await?
        .map_or(true, |value| value == "true");
    let retrieval_query = if use_query_enhancement {
        rag_pipeline::enhance_query(&query)
    } else {
        query.clone()
    };

    let query_embedding = {
        let embedder = embedder.clone();
        let retrieval_query = retrieval_query.clone();
        tokio::task::spawn_blocking(move || rag_pipeline::embed_query(&embedder, &retrieval_query))
            .await
            .map_err(|e| AppError::Internal(format!("Embedding task panicked: {e}")))?
    }?;

    let search = vector_db
        .hybrid_search(&retrieval_query, &query_embedding, 10)
        .await?;
    if search.vector_only {
        let _ = channel.send(TokenEvent::Warning {
            message: "Full-text retrieval is temporarily unavailable; this answer uses vector-only retrieval.".into(),
        });
    }
    let search_results = search.results;

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;

    let mut citations = Vec::new();
    for result in &search_results {
        let Some(doc) = repo.get_by_id(&result.document_id).await? else {
            continue;
        };
        if doc.status != DocumentStatus::Indexed {
            continue;
        }

        citations.push(Citation {
            document_id: result.document_id.clone(),
            filename: doc.filename,
            storage_path: app_local_data.join(&doc.storage_path).to_string_lossy().to_string(),
            page: result.page as i32,
            chunk_content: result.content.clone(),
            score: 1.0 / (1.0 + result.score),
        });
    }

    let context = rag_pipeline::build_rag_prompt(&query, &citations);
    let system_prompt = "You are a helpful assistant that answers questions using only the provided document excerpts. Answer concisely. Cite sources by indicating the page number in brackets like [Page N]. Do not use your own knowledge. Do not use <|think|> or </|think|> tags.";

    let mode = settings_repo.get("inference_mode").await
        .unwrap_or_default()
        .unwrap_or_else(|| "local".to_string());

    if mode == "external" {
        let model = settings_repo.get("external_model").await
            .unwrap_or_default()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_EXTERNAL_MODEL.to_string());

        let base_url = settings_repo.get("external_base_url").await
            .unwrap_or_default()
            .unwrap_or_else(|| "http://localhost:11434/v1".to_string());

        let api_key = tokio::task::spawn_blocking(|| {
            keyring::v1::Entry::new("ragtag", "external_api_key")
                .ok()
                .and_then(|e| e.get_password().ok())
        })
        .await
        .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
        .unwrap_or_default();

        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": model,

            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": context}
            ],
            "stream": true
        });

        let mut req = http_client.post(&url).json(&body);
        if !api_key.is_empty() {
            req = req.bearer_auth(&api_key);
        }

        let response = tokio::select! {
            _ = cancel_receiver.changed() => return Ok(()),
            result = req.send() => result,
        }
        .map_err(|e| AppError::Inference(format!("BYO request failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Inference(format!("BYO request failed: {e}")))?;

        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        loop {
            let chunk = tokio::select! {
                _ = cancel_receiver.changed() => return Ok(()),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else {
                let _ = channel.send(TokenEvent::Error {
                    message: "External inference stream ended before its completion marker.".into(),
                    citations,
                });
                return Ok(());
            };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    let _ = channel.send(TokenEvent::Error {
                        message: format!("Stream error: {e}"),
                        citations,
                    });
                    return Ok(());
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some((pos, delimiter_len)) = buffer
                .find("\r\n\r\n")
                .map(|pos| (pos, 4))
                .or_else(|| buffer.find("\n\n").map(|pos| (pos, 2)))
            {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer = buffer[pos + delimiter_len..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        let _ = channel.send(TokenEvent::Done { citations });
                        return Ok(());
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(token) = parsed["choices"][0]["delta"]["content"].as_str() {
                            if !token.is_empty()
                                && channel.send(TokenEvent::Token { text: token.to_string() }).is_err()
                            {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    let model = {
        let guard = local_llm.read().await;
        guard.clone()
    };

    if let Some(model) = model {
        let request = mistralrs::RequestBuilder::new()
            .add_message(mistralrs::TextMessageRole::System, system_prompt.to_string())
            .add_message(mistralrs::TextMessageRole::User, context);

        let stream_result = tokio::select! {
            _ = cancel_receiver.changed() => return Ok(()),
            result = model.stream_chat_request(request) => result,
        };
        match stream_result {
            Ok(mut stream) => loop {
                let response = tokio::select! {
                    _ = cancel_receiver.changed() => return Ok(()),
                    response = stream.next() => response,
                };
                match response {
                    Some(mistralrs::Response::Chunk(chunk)) => {
                        if let Some(choice) = chunk.choices.first() {
                            if let Some(token) = &choice.delta.content {
                                if !token.is_empty()
                                    && channel.send(TokenEvent::Token { text: token.clone() }).is_err()
                                {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    Some(mistralrs::Response::Done(_response)) => {
                        let _ = channel.send(TokenEvent::Done { citations });
                        return Ok(());
                    }
                    Some(mistralrs::Response::ModelError(message, _)) => {
                        let _ = channel.send(TokenEvent::Error { message, citations });
                        return Ok(());
                    }
                    Some(mistralrs::Response::InternalError(error))
                    | Some(mistralrs::Response::ValidationError(error)) => {
                        let _ = channel.send(TokenEvent::Error {
                            message: error.to_string(),
                            citations,
                        });
                        return Ok(());
                    }
                    None => {
                        let _ = channel.send(TokenEvent::Error {
                            message: "Local inference stream ended before completion.".into(),
                            citations,
                        });
                        return Ok(());
                    }
                    Some(_) => {}
                }
            },
            Err(e) => {
                let _ = channel.send(TokenEvent::Error {
                    message: format!("LLM request failed: {e}"),
                    citations,
                });
            }
        }
    } else {
        let _ = channel.send(TokenEvent::Error {
            message: "No LLM model loaded. Switch to external inference in Settings.".into(),
            citations,
        });
    }

    Ok(())
}

#[tauri::command]
async fn list_documents(state: tauri::State<'_, AppState>) -> Result<Vec<Document>, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    repo.list_all().await
}

#[tauri::command]
async fn delete_document(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), AppError> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::InvalidInput("Document ID must be a UUID".into()))?
        .to_string();
    let _document_operation = state.document_operations.write().await;
    let repo = DocumentRepository::new(state.db_pool.clone());
    let document = repo.get_by_id(&id).await?;
    let pdf_path = if let Some(doc) = document.as_ref() {
        let app_local_data = app.path().app_local_data_dir().map_err(|e| AppError::Io(e.to_string()))?;
        let stored_path = std::path::Path::new(&doc.storage_path);
        if stored_path.is_absolute()
            || stored_path.components().any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(AppError::Internal("Document has an unsafe storage path".into()));
        }
        Some(app_local_data.join(stored_path))
    } else {
        None
    };

    state.vector_db.delete_chunks(&id).await?;
    repo.delete_by_id(&id).await?;

    if let Some(pdf_path) = pdf_path {
        if tokio::fs::metadata(&pdf_path).await.is_ok() {
            tokio::fs::remove_file(pdf_path).await.map_err(|e| AppError::Io(e.to_string()))?;
        }
    }
    let _ = app.emit("document_deleted", serde_json::json!({
        "document_id": &id,
    }));

    Ok(())
}

#[tauri::command]
async fn cancel_query(
    state: tauri::State<'_, AppState>,
    request_id: String,
) -> Result<(), AppError> {
    let queries = state
        .active_queries
        .lock()
        .map_err(|_| AppError::Internal("Query cancellation state is poisoned".into()))?;
    if let Some(sender) = queries.get(&request_id) {
        let _ = sender.send(true);
    }
    Ok(())
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    
    let mut settings = HashMap::new();
    let keys = vec![
        "inference_mode", "external_base_url", "external_model", "generation_model",
        "tokenizer_model", "gguf_filename", "use_query_enhancement", "chunk_size",
        "chunk_overlap"
    ];

    
    for key in keys {
        if let Some(val) = repo.get(key).await? {
            settings.insert(key.to_string(), val);
        }
    }
    
    Ok(settings)
}

#[tauri::command]
async fn set_setting(
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    validate_setting(&key, &value)?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let chunk_size = if key == "chunk_size" {
        value.parse::<usize>()
            .map_err(|_| AppError::InvalidInput("Invalid chunk_size".into()))?
    } else {
        repo.get("chunk_size").await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(512)
    };
    let chunk_overlap = if key == "chunk_overlap" {
        value.parse::<usize>()
            .map_err(|_| AppError::InvalidInput("Invalid chunk_overlap".into()))?
    } else {
        repo.get("chunk_overlap").await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(80)
    };
    if chunk_overlap >= chunk_size {
        return Err(AppError::InvalidInput(
            "chunk_overlap must be smaller than chunk_size".into(),
        ));
    }

    repo.set(&key, &value).await
}

#[tauri::command]
async fn set_api_key(key: String) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| AppError::Internal(format!("keyring: {e}")))?;
        entry.set_password(&key)
            .map_err(|e| AppError::Internal(format!("keyring set: {e}")))?;
        Ok(true)
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
}

#[tauri::command]
async fn has_api_key() -> Result<bool, AppError> {
    tokio::task::spawn_blocking(|| {
        match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => match entry.get_password() {
                Ok(pwd) => Ok(!pwd.is_empty()),
                Err(_) => Ok(false),
            },
            Err(_) => Ok(false),
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
}

#[tauri::command]
async fn delete_api_key() -> Result<bool, AppError> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| AppError::Internal(format!("keyring: {e}")))?;
        entry.delete_credential()
            .map_err(|e| AppError::Internal(format!("keyring delete: {e}")))?;
        Ok(true)
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
}

#[tauri::command]
async fn get_storage_info(
    app: tauri::AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app.path().app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;

    let db_size = tokio::fs::metadata(app_local_data.join("app.db"))
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let lancedb_dir = app_local_data.join(".lancedb");
    let lancedb_size = tokio::task::spawn_blocking({
        let d = lancedb_dir.clone();
        move || dir_size(&d)
    })
    .await
    .unwrap_or(0);

    let docs_dir = app_local_data.join("documents");
    let docs_size = tokio::task::spawn_blocking({
        let d = docs_dir.clone();
        move || dir_size(&d)
    })
    .await
    .unwrap_or(0);

    let models_dir = app_local_data.join("models");
    let models_size = tokio::task::spawn_blocking({
        let d = models_dir.clone();
        move || dir_size(&d)
    })
    .await
    .unwrap_or(0);

    Ok(serde_json::json!({
        "database_bytes": db_size,
        "vector_store_bytes": lancedb_size,
        "documents_bytes": docs_size,
        "models_bytes": models_size,
    }))
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

#[tauri::command]
async fn compact_vector_store(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    state.vector_db.compact().await?;
    Ok(serde_json::json!({"status": "compacted"}))
}

#[tauri::command]
async fn vacuum_database(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    sqlx::query("VACUUM")
        .execute(&state.db_pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(serde_json::json!({"status": "vacuumed"}))
}

#[tauri::command]
async fn reset_all_data(
    app: tauri::AppHandle,
    delete_models: bool,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app.path().app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;

    let sentinel = app_local_data.join(".reset_pending");
    let payload = serde_json::json!({"delete_models": delete_models});
    tokio::fs::write(&sentinel, payload.to_string()).await?;

    info!("reset requested — restarting application");
    app.request_restart();

    Ok(serde_json::json!({"restarting": true}))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            // Two-phase reset: check for pending reset sentinel before any DB init
            let sentinel = app_local_data.join(".reset_pending");
            if sentinel.exists() {
                if let Ok(contents) = std::fs::read_to_string(&sentinel) {
                    if let Ok(config) = serde_json::from_str::<serde_json::Value>(&contents) {
                        let delete_models = config["delete_models"].as_bool().unwrap_or(false);

                        // Purge data directories
                        let _ = std::fs::remove_dir_all(app_local_data.join("documents"));
                        let _ = std::fs::remove_dir_all(app_local_data.join(".lancedb"));
                        let _ = std::fs::remove_file(app_local_data.join("app.db"));
                        let _ = std::fs::remove_file(app_local_data.join("app.db-wal"));
                        let _ = std::fs::remove_file(app_local_data.join("app.db-shm"));

                        if delete_models {
                            // Remove model dirs but preserve fastembed cache
                            if let Ok(entries) = std::fs::read_dir(app_local_data.join("models")) {
                                for entry in entries.flatten() {
                                    let name = entry.file_name();
                                    let name_str = name.to_string_lossy();
                                    if !name_str.contains("fastembed") {
                                        let _ = std::fs::remove_dir_all(entry.path());
                                    }
                                }
                            }
                        }

                        let _ = std::fs::remove_file(&sentinel);
                    }
                }
            }

            let pool = tauri::async_runtime::block_on(init_pool(&app_local_data))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            let embedder: Arc<tokio::sync::RwLock<Option<Arc<std::sync::Mutex<fastembed::TextEmbedding>>>>> =
                Arc::new(tokio::sync::RwLock::new(None));

            let (tx, mut rx) = mpsc::channel::<String>(32);

            let lancedb_dir = app_local_data.join(".lancedb");
            let vector_db = tauri::async_runtime::block_on(VectorDb::init(&lancedb_dir))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            let vector_db = Arc::new(vector_db);

            let local_llm = Arc::new(tokio::sync::RwLock::new(None));
            let document_operations = Arc::new(tokio::sync::RwLock::new(()));
            let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx,
                embedder: embedder.clone(),
                vector_db: vector_db.clone(),
                local_llm: local_llm.clone(),
                document_operations: document_operations.clone(),
                active_queries: active_queries.clone(),
                http_client: reqwest::Client::builder()
                    .use_rustls_tls()
                    .build()
                    .map_err(|e| Box::new(std::io::Error::other(e.to_string())))?,
            });

            let docs_dir = app_local_data.clone();
            let app_handle = app.handle().clone();

            // Background worker: processes documents from the channel
            let embedder_worker = embedder.clone();
            let document_operations_worker = document_operations.clone();
            tauri::async_runtime::spawn(async move {
                let repo = DocumentRepository::new(pool.clone());
                while let Some(doc_id) = rx.recv().await {
                    info!("processing document {doc_id}");

                    let embedder = {
                        let guard = embedder_worker.read().await;
                        guard.clone()
                    };
                    let Some(embedder) = embedder else {
                        warn!("embedder not ready — deferring document {doc_id} (will resume on next restart)");
                        continue;
                    };

                    let _document_operation = document_operations_worker.read().await;


                    let result = match repo.get_by_id(&doc_id).await {
                        Ok(Some(doc)) => {
                            let settings_repo = AppSettingsRepository::new(pool.clone());
                            let chunk_size: usize = settings_repo.get("chunk_size").await
                                .unwrap_or_default()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(512);
                            let chunk_overlap: usize = settings_repo.get("chunk_overlap").await
                                .unwrap_or_default()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(80);
                            let pdf_path = docs_dir.join(&doc.storage_path);
                            let app_for_progress = app_handle.clone();
                            let doc_id_for_progress = doc_id.clone();
                            rag_pipeline::process_document(
                                &pdf_path,
                                &doc_id,
                                chunk_size,
                                chunk_overlap,
                                embedder,
                                &repo,
                                &vector_db,
                                move |stage| {
                                    let _ = app_for_progress.emit("ingestion_progress", serde_json::json!({
                                        "document_id": &doc_id_for_progress,
                                        "stage": stage,
                                    }));
                                },
                            ).await
                        }
                        Ok(None) => {
                            warn!("document {doc_id} not found in DB — may have been deleted");
                            continue;
                        }
                        Err(e) => {
                            error!("database error while fetching document {doc_id}: {e}");
                            continue;
                        }
                    };

                    match result {
                        Ok(()) => {
                            info!("document {doc_id} indexed successfully");
                            let _ = app_handle.emit("document_indexed", serde_json::json!({
                                "document_id": &doc_id,
                                "status": "indexed",
                            }));
                        }
                        Err(e) => {
                            error!("document {doc_id} processing failed: {e}");
                            let _ = repo.mark_failed(&doc_id, &e.to_string()).await;
                            let _ = app_handle.emit("document_failed", serde_json::json!({
                                "document_id": &doc_id,
                                "error": e.to_string(),
                            }));
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            begin_first_run_init, 
            upload_document, 
            execute_query,
            cancel_query,
            list_documents,
            delete_document,
            get_settings,
            set_setting,
            set_api_key,
            has_api_key,
            delete_api_key,
            get_storage_info,
            compact_vector_store,
            vacuum_database,
            reset_all_data
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::validate_setting;

    #[test]
    fn validates_external_model() {
        assert!(validate_setting("external_model", "gpt-4o-mini").is_ok());
        assert!(validate_setting("external_model", "").is_err());
        assert!(validate_setting("external_model", "   ").is_err());
    }

    #[test]
    fn validates_tokenizer_model() {
        assert!(validate_setting("tokenizer_model", "Qwen/Qwen3-1.7B").is_ok());
        for invalid in ["", "Qwen", "Qwen/", "/Qwen3-1.7B", "Qwen/Qwen3/1.7B", "Qwen /Qwen3-1.7B"] {
            assert!(validate_setting("tokenizer_model", invalid).is_err());
        }
    }
}
