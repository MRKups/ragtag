mod state;
pub use state::AppState;
mod model_download;

use domain::{AppError, Citation, Document, DocumentStatus, TokenEvent};
use persistence::{init_pool, DocumentRepository, AppSettingsRepository, VectorDb};
use sha2::{Digest, Sha512};
use std::sync::Arc;
use std::collections::HashMap;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;

#[tauri::command]
fn ping() -> String {
    "pong".to_string()
}

#[tauri::command]
async fn upload_document(
    app: tauri::AppHandle,
    filename: String,
    file_bytes: Vec<u8>,
) -> Result<String, AppError> {
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

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.pdf", &doc_id);
    let storage_path = docs_dir.join(&storage_filename);

    tokio::fs::write(&storage_path, &file_bytes).await?;

    let relative_storage_path = format!("documents/{}", storage_filename);

    // Format matches SQLite CURRENT_TIMESTAMP
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

    repo.insert(&doc).await?;

    state
        .job_queue
        .try_send(doc_id.clone())
        .map_err(|_| AppError::QueueFull)?;

    Ok(doc_id)
}

#[tauri::command]
async fn execute_query(
    _app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    query: String,
    channel: tauri::ipc::Channel<TokenEvent>,
) -> Result<(), AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    let embedder = state.embedder.clone();
    let vector_db = state.vector_db.clone();
    let local_llm = state.local_llm.clone();

    let query_embedding = {
        let embedder = embedder.clone();
        let q = query.clone();
        tokio::task::spawn_blocking(move || rag_pipeline::embed_query(&embedder, &q))
            .await
            .map_err(|e| AppError::Internal(format!("Embedding task panicked: {e}")))?
    }?;

    let search_results = vector_db
        .hybrid_search(&query, &query_embedding, 10)
        .await?;

    let mut citations: Vec<Citation> = Vec::new();
    for result in &search_results {
        if let Ok(Some(doc)) = repo.get_by_id(&result.document_id).await {
            citations.push(Citation {
                document_id: result.document_id.clone(),
                filename: doc.filename.clone(),
                page: result.page as i32,
                chunk_content: result.content.clone(),
            });
        }
    }

    let context = rag_pipeline::build_rag_prompt(&query, &search_results);

    let model = {
        let guard = local_llm.read().await;
        guard.clone()
    };

    if let Some(model) = model {
        let request = mistralrs::RequestBuilder::new()
            .add_message(mistralrs::TextMessageRole::System, 
                "You are a helpful assistant that answers questions using document excerpts. Answer concisely using only the provided context. Do not use your own knowledge.".to_string())
            .add_message(mistralrs::TextMessageRole::User, context);

        let mut stream = model
            .stream_chat_request(request)
            .await
            .map_err(|e| AppError::Inference(format!("LLM request failed: {e}")))?;

        loop {
            match stream.next().await {
                Some(mistralrs::Response::Chunk(chunk)) => {
                    if let Some(choice) = chunk.choices.first() {
                        if let Some(token) = &choice.delta.content {
                            if !token.is_empty() {
                                let _ = channel.send(TokenEvent::Token { text: token.clone() });
                            }
                        }
                    }
                }
                Some(mistralrs::Response::Done(_response)) => {
                    let _ = channel.send(TokenEvent::Done { citations });
                    break;
                }
                Some(mistralrs::Response::ModelError(msg, _)) => {
                    let _ = channel.send(TokenEvent::Error { message: msg });
                    break;
                }
                None => {
                    let _ = channel.send(TokenEvent::Done { citations });
                    break;
                }
                _ => {}
            }
        }
    } else {
        let _ = channel.send(TokenEvent::Error {
            message: "No LLM model loaded. Is BYO inference configured?".into(),
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
    let repo = DocumentRepository::new(state.db_pool.clone());
    
    if let Some(doc) = repo.get_by_id(&id).await? {
        let app_local_data = app.path().app_local_data_dir().map_err(|e| AppError::Io(e.to_string()))?;
        let pdf_path = app_local_data.join(&doc.storage_path);
        
        if pdf_path.exists() {
            tokio::fs::remove_file(pdf_path).await.map_err(|e| AppError::Io(e.to_string()))?;
        }
    }
    
    state.vector_db.delete_chunks(&id).await?;
    repo.delete_by_id(&id).await?;
    
    Ok(())
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    
    let mut settings = HashMap::new();
    let keys = vec![
        "inference_mode", "external_base_url", "generation_model", 
        "isq_level", "use_query_enhancement", "chunk_size", "chunk_overlap"
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
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    repo.set(&key, &value).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            let pool = tauri::async_runtime::block_on(init_pool(&app_local_data))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            let models_dir = app_local_data.join("models");

            let embedder = tauri::async_runtime::block_on(async {
                let dir = models_dir.clone();
                tokio::task::spawn_blocking(move || rag_pipeline::init_embedder(&dir))
                    .await
                    .map_err(|e| std::io::Error::other(format!("Embedder task panicked: {e}")))?
                    .map_err(|e| std::io::Error::other(e.to_string()))
            })?;

            let (tx, mut rx) = mpsc::channel::<String>(32);

            // Crash recovery: reset embedding → queued
            let repo = DocumentRepository::new(pool.clone());
            let recovered = tauri::async_runtime::block_on(repo.reset_embedding_to_queued())
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            for id in recovered {
                let _ = tx.try_send(id);
            }

            // Re-queue any documents that were 'queued' before crash
            let queued = tauri::async_runtime::block_on(repo.get_queued_ids())
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            for id in queued {
                let _ = tx.try_send(id);
            }

            let lancedb_dir = app_local_data.join(".lancedb");
            let vector_db = tauri::async_runtime::block_on(VectorDb::init(&lancedb_dir))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            let vector_db = Arc::new(vector_db);

            let embedder = Arc::new(std::sync::Mutex::new(embedder));

            let local_llm = Arc::new(tokio::sync::RwLock::new(None));

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx,
                embedder: embedder.clone(),
                vector_db: vector_db.clone(),
                local_llm: local_llm.clone(),
            });

            let docs_dir = app_local_data.clone();
            let app_handle = app.handle().clone();

            // Background init for LLM
            let llm_handle = app_handle.clone();
            let llm_state = local_llm.clone();
            let app_settings = persistence::AppSettingsRepository::new(pool.clone());
            let models_dir_llm = models_dir.clone();
            
            tokio::spawn(async move {
                let mode = app_settings.get("inference_mode").await
                    .unwrap_or_default()
                    .unwrap_or_else(|| "local".to_string());
                
                if mode == "local" {
                    eprintln!("[Init] Starting model download via hf-hub...");
                    
                    let model_id = "mistralai/Mistral-7B-Instruct-v0.1"; // TODO: read from settings
                    
                    // Step 1: Download with progress
                    if let Err(e) = model_download::download_model(
                        model_id,
                        &models_dir_llm,
                        &llm_handle,
                    ).await {
                        eprintln!("[Init] Model download failed: {}", e);
                        let _ = llm_handle.emit("model_download_progress", model_download::ModelDownloadProgress {
                            model_id: model_id.to_string(),
                            current_file: String::new(),
                            file_downloaded: 0,
                            file_total: 0,
                            files_completed: 0,
                            total_files: 0,
                            stage: model_download::DownloadStage::Error { message: e.to_string() },
                        });
                        return;
                    }
                    
                    eprintln!("[Init] Model downloaded, building mistral.rs pipeline...");
                    
                    // Step 2: Build the model from cache
                    let builder = mistralrs::TextModelBuilder::new(model_id)
                        .with_isq(mistralrs::IsqType::Q4K)
                        .from_hf_cache_path(models_dir_llm);
                    
                    match builder.build().await {
                        Ok(model) => {
                            *llm_state.write().await = Some(Arc::new(model));
                            eprintln!("[Init] mistral.rs ready");
                            let _ = llm_handle.emit("llm_ready", ());
                        }
                        Err(e) => {
                            eprintln!("[Init] Failed to build mistral model: {}", e);
                        }
                    }
                }
            });

            let _ = app_handle.emit("embedder_ready", ());

            // Background worker: processes documents from the channel
            tokio::spawn(async move {
                let repo = DocumentRepository::new(pool.clone());
                while let Some(doc_id) = rx.recv().await {
                    eprintln!("[Worker] Processing document: {doc_id}");

                    let result = match repo.get_by_id(&doc_id).await {
                        Ok(Some(doc)) => {
                            let pdf_path = docs_dir.join(&doc.storage_path);
                            rag_pipeline::process_document(
                                &pdf_path,
                                &doc_id,
                                embedder.clone(),
                                &repo,
                                &vector_db,
                            ).await
                        }
                        Ok(None) => {
                            eprintln!("[Worker] Document {doc_id} not found");
                            continue;
                        }
                        Err(e) => {
                            eprintln!("[Worker] DB error for {doc_id}: {e}");
                            continue;
                        }
                    };

                    match result {
                        Ok(()) => {
                            eprintln!("[Worker] Document {doc_id} indexed successfully");
                            let _ = app_handle.emit("document_indexed", serde_json::json!({
                                "document_id": &doc_id,
                                "status": "indexed",
                            }));
                        }
                        Err(e) => {
                            eprintln!("[Worker] Document {doc_id} failed: {e}");
                            let _ = repo.update_status(&doc_id, DocumentStatus::Failed).await;
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
            ping, 
            upload_document, 
            execute_query,
            list_documents,
            delete_document,
            get_settings,
            set_setting
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
