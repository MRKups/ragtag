use domain::{AppError, Citation, DocumentStatus, TokenEvent};
use persistence::lancedb::RetrievalMode;
use persistence::{AppSettingsRepository, DocumentRepository};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;

use crate::inference::{
    InferenceProvider, external_inference_config, forward_inference_events,
    probe_external_inference_readiness_cancellable,
};
use crate::state::AppState;
use persistence::storage::document_storage_path;

pub const MAX_EXTERNAL_QUERY_BYTES: usize = 32 * 1024;
pub const MAX_EXTERNAL_REQUEST_ID_BYTES: usize = 64;
pub const RETRIEVAL_CANDIDATE_LIMIT: usize = 100;
pub const RETRIEVAL_RESULT_LIMIT: usize = 10;

pub fn validate_query_request_id(request_id: &str) -> Result<(), AppError> {
    if request_id.len() > MAX_EXTERNAL_REQUEST_ID_BYTES
        || uuid::Uuid::parse_str(request_id).is_err()
    {
        return Err(AppError::invalid_input(
            "Validate query request ID",
            "Query request ID must be a UUID no longer than 64 bytes",
            "Provide a valid UUID for request_id.",
        ));
    }

    Ok(())
}

pub fn validate_query_request(query: &str, request_id: &str) -> Result<(), AppError> {
    if query.len() > MAX_EXTERNAL_QUERY_BYTES {
        return Err(AppError::invalid_input(
            "Validate query request",
            "Query exceeds the 32 KiB limit",
            "Shorten the query prompt to less than 32 KiB.",
        ));
    }
    validate_query_request_id(request_id)
}

pub fn require_retrieved_citations(citations: &[Citation]) -> Result<(), AppError> {
    if citations.is_empty() {
        return Err(AppError::inference(
            "Execute query",
            "No usable document excerpts were retrieved. Add or wait for indexed documents, then try again.",
            "Upload relevant documents and ensure indexing has completed before querying.",
        ));
    }

    Ok(())
}

pub struct ActiveQuery {
    pub queries: Arc<std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    pub request_id: String,
}

impl Drop for ActiveQuery {
    fn drop(&mut self) {
        if let Ok(mut queries) = self.queries.lock() {
            queries.remove(&self.request_id);
        }
    }
}

pub fn register_active_query(
    queries: &Arc<std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    request_id: &str,
) -> Result<tokio::sync::watch::Receiver<bool>, AppError> {
    if request_id.is_empty() {
        return Err(AppError::invalid_input(
            "Register active query",
            "Query request ID must not be empty",
            "Provide a non-empty request_id.",
        ));
    }

    let (sender, receiver) = tokio::sync::watch::channel(false);
    let mut queries = queries.lock().map_err(|_| {
        AppError::internal(
            "Register active query",
            "Query cancellation state is poisoned",
            "Restart the application to reset query state.",
        )
    })?;
    if queries.contains_key(request_id) {
        return Err(AppError::invalid_input(
            "Register active query",
            "A query with this request ID is already active",
            "Wait for the active query to finish or cancel it before starting another with the same ID.",
        ));
    }
    queries.insert(request_id.to_owned(), sender);
    Ok(receiver)
}

#[tauri::command]
pub async fn execute_query(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    query: String,
    request_id: String,
    channel: tauri::ipc::Channel<TokenEvent>,
) -> Result<(), AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Ordinary)?;
    validate_query_request(&query, &request_id)?;
    let repo = DocumentRepository::new(state.db_pool.clone());
    let cancel_receiver = register_active_query(&state.active_queries, &request_id)?;
    let _active_query = ActiveQuery {
        queries: state.active_queries.clone(),
        request_id: request_id.clone(),
    };
    let settings_repo = AppSettingsRepository::new(state.db_pool.clone());

    let embedder_opt = {
        let guard = state.embedder.read().await;
        guard.clone()
    };
    let embedder = embedder_opt.ok_or_else(|| {
        AppError::internal(
            "Embed query",
            "Embedding model not yet loaded. Please wait.",
            "Wait for the local embedding model to finish loading before executing queries.",
        )
    })?;
    let vector_db = state.vector_db.clone();
    let local_llm = state.local_llm.clone();

    let use_query_enhancement = settings_repo
        .get("use_query_enhancement")
        .await?
        .is_none_or(|value| value == "true");
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
            .map_err(|e| {
                AppError::internal(
                    "Embed query",
                    format!("Embedding task panicked: {e}"),
                    "Restart the application and retry the query.",
                )
            })?
    }?;

    let app_local_data = app.path().app_local_data_dir().map_err(|e| {
        AppError::storage(
            "Resolve storage path",
            format!("Failed to resolve app local data directory: {e}"),
            "Verify application local storage permissions.",
        )
    })?;
    let citations = {
        let _document_operation = state.document_operations.read().await;
        let indexed_document_ids = repo.get_indexed_ids().await?;
        let search = vector_db
            .hybrid_search(
                &retrieval_query,
                &query_embedding,
                &indexed_document_ids,
                RETRIEVAL_CANDIDATE_LIMIT,
            )
            .await?;
        let is_hybrid = search.retrieval_mode == RetrievalMode::Hybrid;
        if search.retrieval_mode == RetrievalMode::VectorOnly {
            let _ = channel.send(TokenEvent::Warning {
                message: "Full-text retrieval is temporarily unavailable; this answer uses vector-only retrieval.".into(),
            });
        }

        let mut citations = Vec::new();
        for result in search.results {
            let Some(doc) = repo.get_by_id(&result.document_id).await? else {
                continue;
            };
            if doc.status != DocumentStatus::Indexed || result.content.trim().is_empty() {
                continue;
            }

            let doc_storage_path = doc.storage_path.clone();
            let app_data = app_local_data.clone();
            let storage_path = tokio::task::spawn_blocking(move || {
                document_storage_path(&app_data, &doc_storage_path)
            })
            .await
            .map_err(|e| {
                AppError::internal(
                    "Resolve citation path",
                    format!("Storage path check task panicked: {e}"),
                    "Restart the application and retry the query.",
                )
            })??;

            citations.push(Citation {
                document_id: result.document_id,
                filename: doc.filename,
                storage_path: storage_path.to_string_lossy().to_string(),
                page: result.page as i32,
                chunk_content: result.content,
                score: 1.0 / (1.0 + result.score),
                is_hybrid,
            });
            if citations.len() == RETRIEVAL_RESULT_LIMIT {
                break;
            }
        }
        citations
    };
    require_retrieved_citations(&citations)?;

    if *cancel_receiver.borrow() {
        return Ok(());
    }

    let context = rag_pipeline::build_rag_prompt(&query, &citations);
    let system_prompt = "You are a helpful assistant that answers questions using only the provided document excerpts. Answer concisely. You MUST ground every statement by citing the corresponding source numbers in brackets, for example [1] or [2] (or multiple like [1][2]), matching the numbered sources provided in the context. Place citations at the end of every relevant sentence or bullet point (e.g. \"...enforces policy execution [1].\"). Do not write out words like \"Source\" or \"Page\" inside the brackets; use only the bracketed numbers like [1]. Do not invent citations or page numbers outside the provided excerpts. Do not use your own knowledge. Do not use <|think|> or </|think|> tags.";

    let mode = settings_repo
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    let provider = if mode == "external" {
        let config = external_inference_config(&settings_repo).await?;
        let http_client = state.http_client.clone();
        let mut readiness_cancellation = cancel_receiver.clone();
        if !probe_external_inference_readiness_cancellable(
            &http_client,
            &config,
            &mut readiness_cancellation,
        )
        .await?
        {
            return Ok(());
        }
        let url = config.chat_completions_url()?.to_string();
        let model = config.model;

        InferenceProvider::External {
            client: http_client,
            url,
            body: serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": context}
                ],
                "stream": true
            }),
            api_key: config.api_key,
        }
    } else {
        let model = {
            let guard = local_llm.read().await;
            guard.clone()
        };
        match model {
            Some(model) => InferenceProvider::Local {
                model,
                system_prompt: system_prompt.to_string(),
                context,
            },
            None => InferenceProvider::MissingLocal,
        }
    };

    forward_inference_events(provider, cancel_receiver, channel, citations).await
}

#[tauri::command]
pub async fn cancel_query(
    state: tauri::State<'_, AppState>,
    request_id: String,
) -> Result<(), AppError> {
    let _permit = state
        .capacity
        .acquire(crate::capacity::CommandPriority::Protected)?;
    validate_query_request_id(&request_id)?;
    let queries = state.active_queries.lock().map_err(|_| {
        AppError::internal(
            "Cancel query",
            "Query cancellation state is poisoned",
            "Restart the application to reset query state.",
        )
    })?;
    if let Some(sender) = queries.get(&request_id) {
        let _ = sender.send(true);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_external_query_request() {
        assert!(validate_query_request("question", &uuid::Uuid::now_v7().to_string()).is_ok());
        assert!(validate_query_request("question", "not-a-uuid").is_err());
        assert!(
            validate_query_request(
                &"x".repeat(32 * 1024 + 1),
                &uuid::Uuid::now_v7().to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_queries_without_retrieved_citations() {
        let error = require_retrieved_citations(&[]).unwrap_err();
        assert_eq!(error.code, domain::ErrorCode::Inference);
        let serialized = serde_json::to_value(error).unwrap();
        assert_eq!(serialized["code"], "inference");
        assert!(!serialized["recovery"].as_str().unwrap().is_empty());
    }

    #[test]
    fn rejects_duplicate_active_query_request_ids() {
        let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));
        assert!(register_active_query(&active_queries, "request-1").is_ok());
        let err = register_active_query(&active_queries, "request-1").unwrap_err();
        assert_eq!(err.code, domain::ErrorCode::InvalidInput);
    }
}
