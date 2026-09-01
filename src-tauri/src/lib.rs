mod state;
pub use state::{AppState, IngestionJob};
mod model_download;

use domain::{
    AppError, Citation, Document, DocumentStatus, IngestionProgress, TokenEvent,
    models::IndexProvenance,
};
use log::{debug, error, info, warn};
use persistence::lancedb::RetrievalMode;
use persistence::{AppSettingsRepository, DocumentRepository, VectorDb, init_pool};
use sha2::{Digest, Sha512};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;

static EMBEDDER_INIT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
const DEFAULT_EXTERNAL_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TOKENIZER_MODEL: &str = "Qwen/Qwen3-1.7B";

const EXTRACTION_VERSION: &str = "token-aware-v2";
const DEFAULT_CHUNK_SIZE: usize = 512;
const DEFAULT_CHUNK_OVERLAP: usize = 80;
const REGISTRY_READ_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const REGISTRY_READ_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
const RETRIEVAL_CANDIDATE_LIMIT: usize = 100;
const RETRIEVAL_RESULT_LIMIT: usize = 10;
const EXTERNAL_READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_EXTERNAL_BASE_URL: &str = "http://localhost:11434/v1";

fn validate_upload(filename: &str, bytes: &[u8]) -> Result<(), AppError> {
    let name = std::path::Path::new(filename);
    if filename.is_empty()
        || filename.len() > 255
        || name.file_name().and_then(|name| name.to_str()) != Some(filename)
        || !filename.to_ascii_lowercase().ends_with(".pdf")
    {
        return Err(AppError::InvalidInput(
            "Upload filename must be a PDF filename".into(),
        ));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::InvalidInput(
            "PDF exceeds the 50 MiB upload limit".into(),
        ));
    }
    let prefix = &bytes[..bytes.len().min(1024)];
    if !prefix.windows(5).any(|window| window == b"%PDF-") {
        return Err(AppError::InvalidInput("Upload is not a PDF".into()));
    }
    Ok(())
}

fn sse_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf_boundary = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    let lf_boundary = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));

    match (crlf_boundary, lf_boundary) {
        (Some(crlf), Some(lf)) => Some(if crlf.0 <= lf.0 { crlf } else { lf }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn sse_event_data(event: &str) -> Option<&str> {
    event.lines().find_map(|line| {
        let line = line.strip_suffix('\r').unwrap_or(line);
        line.strip_prefix("data:")
            .map(|data| data.strip_prefix(' ').unwrap_or(data))
    })
}

const INFERENCE_EVENT_BUFFER_CAPACITY: usize = 32;

#[derive(Clone)]
struct ExternalInferenceConfig {
    base_url: reqwest::Url,
    model: String,
    api_key: Option<String>,
}

impl ExternalInferenceConfig {
    fn endpoint_url(&self, segments: &[&str]) -> Result<reqwest::Url, AppError> {
        let mut url = self.base_url.clone();
        while url.path().ends_with("//") {
            let mut path = url.path_segments_mut().map_err(|_| {
                AppError::InvalidInput(
                    "External base URL cannot be used as an HTTP endpoint".into(),
                )
            })?;
            path.pop_if_empty();
        }
        let mut path = url.path_segments_mut().map_err(|_| {
            AppError::InvalidInput("External base URL cannot be used as an HTTP endpoint".into())
        })?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        Ok(url)
    }

    fn models_url(&self) -> Result<reqwest::Url, AppError> {
        self.endpoint_url(&["models"])
    }

    fn chat_completions_url(&self) -> Result<reqwest::Url, AppError> {
        self.endpoint_url(&["chat", "completions"])
    }
}

#[derive(serde::Deserialize)]
struct ExternalModelsResponse {
    data: Vec<ExternalModel>,
}

#[derive(serde::Deserialize)]
struct ExternalModel {
    id: String,
}

fn normalize_external_base_url(value: &str) -> Result<reqwest::Url, AppError> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| {
        AppError::InvalidInput("External base URL must be a valid HTTP(S) URL".into())
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::InvalidInput(
            "External base URL must be an HTTP(S) URL without credentials, query, or fragment"
                .into(),
        ));
    }

    Ok(url)
}

fn apply_external_auth(
    request: reqwest::RequestBuilder,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    match api_key.filter(|key| !key.is_empty()) {
        Some(api_key) => request.bearer_auth(api_key),
        None => request,
    }
}

async fn read_external_api_key() -> Result<Option<String>, AppError> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| AppError::Internal(format!("keyring: {e}")))?;
        match entry.get_password() {
            Ok(api_key) if !api_key.is_empty() => Ok(Some(api_key)),
            Ok(_) | Err(_) => Ok(None),
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
}

async fn external_inference_config(
    settings: &AppSettingsRepository,
) -> Result<ExternalInferenceConfig, AppError> {
    let base_url = settings
        .get("external_base_url")
        .await?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EXTERNAL_BASE_URL.to_string());
    let model = settings
        .get("external_model")
        .await?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EXTERNAL_MODEL.to_string());

    Ok(ExternalInferenceConfig {
        base_url: normalize_external_base_url(&base_url)?,
        model: model.trim().to_string(),
        api_key: read_external_api_key().await?,
    })
}

fn external_model_is_available(models: &ExternalModelsResponse, configured_model: &str) -> bool {
    models
        .data
        .iter()
        .any(|available_model| available_model.id == configured_model)
}

async fn probe_external_inference_readiness(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
) -> Result<(), AppError> {
    let models_url = config.models_url()?;
    let response = apply_external_auth(
        client.get(models_url).timeout(EXTERNAL_READINESS_TIMEOUT),
        config.api_key.as_deref(),
    )
    .send()
    .await
    .map_err(|error| {
        AppError::Inference(format!(
            "External inference readiness check failed: {error}"
        ))
    })?;

    if !response.status().is_success() {
        return Err(AppError::Inference(format!(
            "External inference readiness check failed: endpoint returned HTTP {}",
            response.status()
        )));
    }

    let models = response
        .json::<ExternalModelsResponse>()
        .await
        .map_err(|error| {
            AppError::Inference(format!(
                "External inference readiness check received an invalid /models response: {error}"
            ))
        })?;
    if !external_model_is_available(&models, &config.model) {
        return Err(AppError::Inference(format!(
            "External inference model '{}' is not available from the configured endpoint",
            config.model
        )));
    }

    Ok(())
}

enum InferenceEvent {
    Token(String),
    Done,
    StreamError(String),
    StartupError(AppError),
}

enum InferenceProvider {
    External {
        client: reqwest::Client,
        url: String,
        body: serde_json::Value,
        api_key: Option<String>,
    },
    Local {
        model: Arc<mistralrs::Model>,
        system_prompt: String,
        context: String,
    },
    MissingLocal,
}

impl InferenceProvider {
    async fn stream(
        self,
        mut cancellation: tokio::sync::watch::Receiver<bool>,
        events: mpsc::Sender<InferenceEvent>,
    ) -> Result<(), AppError> {
        match self {
            Self::External {
                client,
                url,
                body,
                api_key,
            } => {
                stream_external_inference(client, url, body, api_key, &mut cancellation, &events)
                    .await
            }
            Self::Local {
                model,
                system_prompt,
                context,
            } => {
                stream_local_inference(model, system_prompt, context, &mut cancellation, &events)
                    .await
            }
            Self::MissingLocal => {
                send_inference_event(
                    &mut cancellation,
                    &events,
                    InferenceEvent::StreamError(
                        "No LLM model loaded. Switch to external inference in Settings.".into(),
                    ),
                )
                .await;
                Ok(())
            }
        }
    }
}

async fn send_inference_event(
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
    event: InferenceEvent,
) -> bool {
    tokio::select! {
        _ = cancellation.changed() => false,
        result = events.send(event) => result.is_ok(),
    }
}

async fn stream_external_inference(
    client: reqwest::Client,
    url: String,
    body: serde_json::Value,
    api_key: Option<String>,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    use futures_util::StreamExt;

    let request = apply_external_auth(client.post(url).json(&body), api_key.as_deref());

    let response = tokio::select! {
        _ = cancellation.changed() => return Ok(()),
        result = request.send() => result,
    }
    .map_err(|e| AppError::Inference(format!("BYO request failed: {e}")))?
    .error_for_status()
    .map_err(|e| AppError::Inference(format!("BYO request failed: {e}")))?;

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    loop {
        let chunk = tokio::select! {
            _ = cancellation.changed() => return Ok(()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            send_inference_event(
                cancellation,
                events,
                InferenceEvent::StreamError(
                    "External inference stream ended before its completion marker.".into(),
                ),
            )
            .await;
            return Ok(());
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                send_inference_event(
                    cancellation,
                    events,
                    InferenceEvent::StreamError(format!("Stream error: {error}")),
                )
                .await;
                return Ok(());
            }
        };
        buffer.extend_from_slice(&chunk);

        while let Some((event_end, delimiter_len)) = sse_event_boundary(&buffer) {
            let event = match std::str::from_utf8(&buffer[..event_end]) {
                Ok(event) => event,
                Err(_) => {
                    send_inference_event(
                        cancellation,
                        events,
                        InferenceEvent::StreamError(
                            "External inference sent invalid UTF-8 in a complete event.".into(),
                        ),
                    )
                    .await;
                    return Ok(());
                }
            };
            let data = sse_event_data(event).map(str::to_owned);
            buffer.drain(..event_end + delimiter_len);

            match data.as_deref() {
                Some("[DONE]") => {
                    send_inference_event(cancellation, events, InferenceEvent::Done).await;
                    return Ok(());
                }
                Some(data) => {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(token) = parsed["choices"][0]["delta"]["content"].as_str() {
                            if !token.is_empty()
                                && !send_inference_event(
                                    cancellation,
                                    events,
                                    InferenceEvent::Token(token.to_string()),
                                )
                                .await
                            {
                                return Ok(());
                            }
                        }
                    }
                }
                None => {}
            }
        }
    }
}

async fn stream_local_inference(
    model: Arc<mistralrs::Model>,
    system_prompt: String,
    context: String,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    let request = mistralrs::RequestBuilder::new()
        .add_message(mistralrs::TextMessageRole::System, system_prompt)
        .add_message(mistralrs::TextMessageRole::User, context);
    let stream_result = tokio::select! {
        _ = cancellation.changed() => return Ok(()),
        result = model.stream_chat_request(request) => result,
    };
    let mut stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => {
            send_inference_event(
                cancellation,
                events,
                InferenceEvent::StreamError(format!("LLM request failed: {error}")),
            )
            .await;
            return Ok(());
        }
    };

    loop {
        let response = tokio::select! {
            _ = cancellation.changed() => return Ok(()),
            response = stream.next() => response,
        };
        match response {
            Some(mistralrs::Response::Chunk(chunk)) => {
                if let Some(token) = chunk
                    .choices
                    .first()
                    .and_then(|choice| choice.delta.content.as_ref())
                    .filter(|token| !token.is_empty())
                {
                    if !send_inference_event(
                        cancellation,
                        events,
                        InferenceEvent::Token(token.clone()),
                    )
                    .await
                    {
                        return Ok(());
                    }
                }
            }
            Some(mistralrs::Response::Done(_)) => {
                send_inference_event(cancellation, events, InferenceEvent::Done).await;
                return Ok(());
            }
            Some(mistralrs::Response::ModelError(message, _)) => {
                send_inference_event(cancellation, events, InferenceEvent::StreamError(message))
                    .await;
                return Ok(());
            }
            Some(mistralrs::Response::InternalError(error))
            | Some(mistralrs::Response::ValidationError(error)) => {
                send_inference_event(
                    cancellation,
                    events,
                    InferenceEvent::StreamError(error.to_string()),
                )
                .await;
                return Ok(());
            }
            None => {
                send_inference_event(
                    cancellation,
                    events,
                    InferenceEvent::StreamError(
                        "Local inference stream ended before completion.".into(),
                    ),
                )
                .await;
                return Ok(());
            }
            Some(_) => {}
        }
    }
}

async fn forward_inference_events(
    provider: InferenceProvider,
    mut cancellation: tokio::sync::watch::Receiver<bool>,
    channel: tauri::ipc::Channel<TokenEvent>,
    citations: Vec<Citation>,
) -> Result<(), AppError> {
    let (events, mut receiver) = mpsc::channel(INFERENCE_EVENT_BUFFER_CAPACITY);
    let producer_cancellation = cancellation.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = provider.stream(producer_cancellation, events.clone()).await {
            let _ = events.send(InferenceEvent::StartupError(error)).await;
        }
    });

    loop {
        tokio::select! {
            biased;
            _ = cancellation.changed() => return Ok(()),
            event = receiver.recv() => match event {
                Some(InferenceEvent::Token(text)) => {
                    if channel.send(TokenEvent::Token { text }).is_err() {
                        return Ok(());
                    }
                }
                Some(InferenceEvent::Done) => {
                    let _ = channel.send(TokenEvent::Done { citations });
                    return Ok(());
                }
                Some(InferenceEvent::StreamError(message)) => {
                    let _ = channel.send(TokenEvent::Error { message, citations });
                    return Ok(());
                }
                Some(InferenceEvent::StartupError(error)) => return Err(error),
                None => {
                    let _ = channel.send(TokenEvent::Error {
                        message: "Inference provider ended without a terminal event.".into(),
                        citations,
                    });
                    return Ok(());
                }
            }
        }
    }
}

fn start_ingestion_progress_forwarder(
    channel: tauri::ipc::Channel<IngestionProgress>,
) -> mpsc::UnboundedSender<IngestionProgress> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            if channel.send(progress).is_err() {
                return;
            }
        }
    });
    sender
}

fn registry_read_retry_delay(attempt: u32) -> Duration {
    let multiplier = 1_u32 << attempt.saturating_sub(1).min(7);
    REGISTRY_READ_RETRY_BASE_DELAY
        .saturating_mul(multiplier)
        .min(REGISTRY_READ_RETRY_MAX_DELAY)
}

fn is_valid_hugging_face_model_id(value: &str) -> bool {
    value.split_once('/').is_some_and(|(organization, model)| {
        !organization.is_empty()
            && !model.is_empty()
            && !model.contains('/')
            && !value.contains(char::is_whitespace)
    })
}

fn require_retrieved_citations(citations: &[Citation]) -> Result<(), AppError> {
    if citations.is_empty() {
        return Err(AppError::Inference(
            "No usable document excerpts were retrieved. Add or wait for indexed documents, then try again."
                .into(),
        ));
    }

    Ok(())
}

fn validate_setting(key: &str, value: &str) -> Result<(), AppError> {
    match key {
        "inference_mode" if matches!(value, "local" | "external") => Ok(()),
        "use_query_enhancement" if matches!(value, "true" | "false") => Ok(()),
        "chunk_size" => value
            .parse::<usize>()
            .ok()
            .filter(|size| (128..=2048).contains(size))
            .map(|_| ())
            .ok_or_else(|| AppError::InvalidInput("chunk_size must be 128–2048".into())),
        "chunk_overlap" => value
            .parse::<usize>()
            .ok()
            .filter(|size| *size <= 1024)
            .map(|_| ())
            .ok_or_else(|| AppError::InvalidInput("chunk_overlap must be 0–1024".into())),
        "external_base_url" => normalize_external_base_url(value).map(|_| ()),
        "external_model" if !value.trim().is_empty() => Ok(()),

        "generation_model"
            if value.split('/').count() == 2 && !value.contains(char::is_whitespace) =>
        {
            Ok(())
        }
        "tokenizer_model" if is_valid_hugging_face_model_id(value) => Ok(()),
        "gguf_filename"
            if !value.is_empty() && !value.contains('/') && value.ends_with(".gguf") =>
        {
            Ok(())
        }
        _ => Err(AppError::InvalidInput(format!(
            "Unsupported or invalid setting: {key}"
        ))),
    }
}

fn parse_chunk_setting(key: &str, value: &str) -> Result<i32, AppError> {
    validate_setting(key, value)?;
    value
        .parse::<i32>()
        .map_err(|_| AppError::InvalidInput(format!("Invalid {key}")))
}

fn validate_indexing_provenance(provenance: &IndexProvenance) -> Result<(), AppError> {
    if provenance.chunk_overlap >= provenance.chunk_size {
        return Err(AppError::InvalidInput(
            "chunk_overlap must be smaller than chunk_size".into(),
        ));
    }

    Ok(())
}

async fn desired_indexing_provenance(
    settings: &AppSettingsRepository,
) -> Result<IndexProvenance, AppError> {
    let chunk_size = settings
        .get("chunk_size")
        .await?
        .unwrap_or_else(|| DEFAULT_CHUNK_SIZE.to_string());
    let chunk_overlap = settings
        .get("chunk_overlap")
        .await?
        .unwrap_or_else(|| DEFAULT_CHUNK_OVERLAP.to_string());
    let provenance = IndexProvenance {
        extraction_version: EXTRACTION_VERSION.to_string(),
        chunk_size: parse_chunk_setting("chunk_size", &chunk_size)?,
        chunk_overlap: parse_chunk_setting("chunk_overlap", &chunk_overlap)?,
    };
    validate_indexing_provenance(&provenance)?;
    Ok(provenance)
}

fn enqueue_requeued_documents(
    queue: mpsc::Sender<IngestionJob>,
    document_ids: Vec<String>,
) -> Result<(), AppError> {
    let mut remaining = document_ids.into_iter();

    while let Some(document_id) = remaining.next() {
        match queue.try_send(IngestionJob::background(document_id)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(job)) => {
                tauri::async_runtime::spawn(async move {
                    for job in std::iter::once(job).chain(remaining.map(IngestionJob::background)) {
                        if let Err(error) = queue.send(job).await {
                            error!(
                                "stale document reindex recovery producer stopped before enqueue: {error}"
                            );
                            break;
                        }
                    }
                });
                return Err(AppError::QueueFull);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                return Err(AppError::Internal(
                    "document worker stopped before stale document reindex could be queued".into(),
                ));
            }
        }
    }

    Ok(())
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

fn invalidate_llm_initialization(initialization: &mut state::LlmInitializationState) {
    initialization.generation = initialization.generation.wrapping_add(1);
}

async fn finish_llm_initialization(
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
) -> bool {
    let mut current = initialization.lock().await;
    if current.in_progress_generation != Some(generation) {
        return false;
    }

    current.in_progress = false;
    current.in_progress_generation = None;
    if current.generation == generation {
        current.restart_requested = false;
        return false;
    }

    let restart = current.restart_requested;
    current.restart_requested = false;
    restart
}

async fn llm_initialization_is_current(
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
) -> bool {
    let current = initialization.lock().await;
    current.generation == generation && current.in_progress_generation == Some(generation)
}

async fn emit_model_download_progress_if_current(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
    progress: model_download::ModelDownloadProgress,
) -> bool {
    let current = initialization.lock().await;
    if current.generation != generation || current.in_progress_generation != Some(generation) {
        return false;
    }

    let _ = app.emit("model_download_progress", progress);
    true
}

async fn finish_llm_initialization_with_progress(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
    progress: model_download::ModelDownloadProgress,
) -> bool {
    let mut current = initialization.lock().await;
    if current.in_progress_generation != Some(generation) {
        return false;
    }

    current.in_progress = false;
    current.in_progress_generation = None;
    if current.generation != generation {
        let restart = current.restart_requested;
        current.restart_requested = false;
        return restart;
    }

    current.restart_requested = false;
    let _ = app.emit("model_download_progress", progress);
    false
}

async fn finish_external_readiness_probe(
    app: &tauri::AppHandle,
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
    readiness: Result<(), AppError>,
) -> bool {
    let mut current = initialization.lock().await;
    if current.in_progress_generation != Some(generation) {
        return false;
    }

    current.in_progress = false;
    current.in_progress_generation = None;
    if current.generation != generation {
        let restart = current.restart_requested;
        current.restart_requested = false;
        return restart;
    }

    current.restart_requested = false;
    match readiness {
        Ok(()) => {
            info!("external inference endpoint and model are ready");
            let _ = app.emit("llm_ready", ());
        }
        Err(error) => {
            warn!("external inference readiness check failed: {error}");
            let _ = app.emit("external_inference_failed", error);
        }
    }
    false
}

async fn finish_local_model_initialization(
    app: &tauri::AppHandle,
    llm_state: &Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: &Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    generation: u64,
    model: mistralrs::Model,
) -> bool {
    let mut current = initialization.lock().await;
    if current.in_progress_generation != Some(generation) {
        return false;
    }

    if current.generation != generation {
        current.in_progress = false;
        current.in_progress_generation = None;
        let restart = current.restart_requested;
        current.restart_requested = false;
        return restart;
    }

    *llm_state.write().await = Some(Arc::new(model));
    current.in_progress = false;
    current.in_progress_generation = None;
    current.restart_requested = false;
    info!("mistral.rs pipeline built and ready");
    let _ = app.emit("llm_ready", ());
    false
}

async fn restart_external_readiness_probe(
    app: &tauri::AppHandle,
    state: &AppState,
    clear_local_model: bool,
) -> Result<(), AppError> {
    {
        let mut initialization = state.llm_initialization.lock().await;
        invalidate_llm_initialization(&mut initialization);
        initialization.in_progress = false;
        initialization.in_progress_generation = None;
        initialization.restart_requested = false;
        if clear_local_model {
            *state.local_llm.write().await = None;
        }
        let _ = app.emit("llm_unready", ());
    }
    let models_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("models");
    request_llm_initialization(app, state, models_dir).await;
    Ok(())
}

async fn set_local_model_setting_and_restart_if_active(
    app: &tauri::AppHandle,
    state: &AppState,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let should_start = {
        let mut initialization = state.llm_initialization.lock().await;
        let changed = settings.get(key).await?.as_deref() != Some(value);
        settings.set(key, value).await?;
        let local_mode_active = settings.get("inference_mode").await?.as_deref() == Some("local");
        if !changed || !local_mode_active {
            false
        } else {
            invalidate_llm_initialization(&mut initialization);
            let should_start = !initialization.in_progress;
            if !should_start {
                initialization.restart_requested = true;
            }
            *state.local_llm.write().await = None;
            let _ = app.emit("llm_unready", ());
            should_start
        }
    };

    if should_start {
        let models_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|e| AppError::Io(e.to_string()))?
            .join("models");
        request_llm_initialization(app, state, models_dir).await;
    }
    Ok(())
}

async fn restart_local_model_initialization_if_active(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    if settings.get("inference_mode").await?.as_deref() != Some("local") {
        return Ok(());
    }

    let should_start = {
        let mut initialization = state.llm_initialization.lock().await;
        invalidate_llm_initialization(&mut initialization);
        let should_start = !initialization.in_progress;
        if !should_start {
            initialization.restart_requested = true;
        }
        *state.local_llm.write().await = None;
        let _ = app.emit("llm_unready", ());
        should_start
    };

    if should_start {
        let models_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|e| AppError::Io(e.to_string()))?
            .join("models");
        request_llm_initialization(app, state, models_dir).await;
    }
    Ok(())
}

async fn revalidate_external_readiness_if_active(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    if settings.get("inference_mode").await?.as_deref() == Some("external") {
        restart_external_readiness_probe(app, state, false).await?;
    }
    Ok(())
}

async fn request_llm_initialization(
    app: &tauri::AppHandle,
    state: &AppState,
    models_dir: std::path::PathBuf,
) {
    request_llm_initialization_with_resources(
        app.clone(),
        state.local_llm.clone(),
        state.llm_initialization.clone(),
        state.db_pool.clone(),
        state.http_client.clone(),
        models_dir,
    )
    .await;
}

fn schedule_llm_initialization_with_resources(
    app: tauri::AppHandle,
    llm_state: Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    pool: sqlx::SqlitePool,
    http_client: reqwest::Client,
    models_dir: std::path::PathBuf,
) {
    tauri::async_runtime::spawn(async move {
        request_llm_initialization_with_resources(
            app,
            llm_state,
            initialization,
            pool,
            http_client,
            models_dir,
        )
        .await;
    });
}

async fn request_llm_initialization_with_resources(
    app: tauri::AppHandle,
    llm_state: Arc<tokio::sync::RwLock<Option<Arc<mistralrs::Model>>>>,
    initialization: Arc<tokio::sync::Mutex<state::LlmInitializationState>>,
    pool: sqlx::SqlitePool,
    http_client: reqwest::Client,
    models_dir: std::path::PathBuf,
) {
    let generation = {
        let mut current = initialization.lock().await;
        if current.in_progress {
            return;
        }
        current.in_progress = true;
        current.in_progress_generation = Some(current.generation);
        current.generation
    };

    tauri::async_runtime::spawn(async move {
        let app_settings = AppSettingsRepository::new(pool.clone());
        let mode = app_settings
            .get("inference_mode")
            .await
            .unwrap_or_default()
            .unwrap_or_else(|| "local".to_string());

        if mode != "local" {
            let readiness = async {
                let config = external_inference_config(&app_settings).await?;
                probe_external_inference_readiness(&http_client, &config).await
            }
            .await;
            let restart =
                finish_external_readiness_probe(&app, &initialization, generation, readiness).await;
            if restart {
                schedule_llm_initialization_with_resources(
                    app.clone(),
                    llm_state.clone(),
                    initialization.clone(),
                    pool.clone(),
                    http_client.clone(),
                    models_dir.clone(),
                );
            }
            return;
        }

        let local_config = async {
            let model_id = app_settings
                .get("generation_model")
                .await?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "unsloth/Qwen3-1.7B-GGUF".to_string());
            let tokenizer_model = app_settings
                .get("tokenizer_model")
                .await?
                .filter(|model| is_valid_hugging_face_model_id(model))
                .unwrap_or_else(|| DEFAULT_TOKENIZER_MODEL.to_string());
            let gguf_filename = app_settings
                .get("gguf_filename")
                .await?
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Qwen3-1.7B-Q4_K_M.gguf".to_string());
            Ok::<_, AppError>((model_id, tokenizer_model, gguf_filename))
        }
        .await;
        let (model_id, tokenizer_model, gguf_filename) = match local_config {
            Ok(config) => config,
            Err(error) => {
                let message = format!("Failed to read local model configuration: {error}");
                error!("{message}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_download::ModelDownloadProgress {
                        model_id: String::new(),
                        current_file: String::new(),
                        file_downloaded: 0,
                        file_total: 0,
                        files_completed: 0,
                        total_files: 0,
                        stage: model_download::DownloadStage::Error { message },
                    },
                )
                .await;
                if restart {
                    schedule_llm_initialization_with_resources(
                        app.clone(),
                        llm_state.clone(),
                        initialization.clone(),
                        pool.clone(),
                        http_client.clone(),
                        models_dir.clone(),
                    );
                }
                return;
            }
        };

        if !llm_initialization_is_current(&initialization, generation).await {
            let restart = finish_llm_initialization(&initialization, generation).await;
            if restart {
                schedule_llm_initialization_with_resources(
                    app.clone(),
                    llm_state.clone(),
                    initialization.clone(),
                    pool.clone(),
                    http_client.clone(),
                    models_dir.clone(),
                );
            }
            return;
        }

        info!("downloading model '{model_id}' ({gguf_filename}) via hf-hub...");
        let progress_initialization = initialization.clone();
        let progress_app = app.clone();
        let progress_emitter: model_download::ProgressEmitter = Arc::new(move |progress| {
            let Ok(current) = progress_initialization.try_lock() else {
                return;
            };
            if current.generation == generation
                && current.in_progress_generation == Some(generation)
            {
                let _ = progress_app.emit("model_download_progress", progress);
            }
        });
        let gguf_path = match model_download::download_model(
            &model_id,
            &gguf_filename,
            &models_dir,
            progress_emitter,
        )
        .await
        {
            Ok(path) => path,
            Err(error) => {
                error!("model download failed after all retries: {error}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_download::ModelDownloadProgress {
                        model_id: model_id.to_string(),
                        current_file: String::new(),
                        file_downloaded: 0,
                        file_total: 0,
                        files_completed: 0,
                        total_files: 0,
                        stage: model_download::DownloadStage::Error {
                            message: error.to_string(),
                        },
                    },
                )
                .await;
                if restart {
                    schedule_llm_initialization_with_resources(
                        app.clone(),
                        llm_state.clone(),
                        initialization.clone(),
                        pool.clone(),
                        http_client.clone(),
                        models_dir.clone(),
                    );
                }
                return;
            }
        };

        if !emit_model_download_progress_if_current(
            &app,
            &initialization,
            generation,
            model_download::ModelDownloadProgress {
                model_id: model_id.to_string(),
                current_file: String::new(),
                file_downloaded: 0,
                file_total: 0,
                files_completed: 1,
                total_files: 1,
                stage: model_download::DownloadStage::LoadingModel,
            },
        )
        .await
        {
            let restart = finish_llm_initialization(&initialization, generation).await;
            if restart {
                schedule_llm_initialization_with_resources(
                    app.clone(),
                    llm_state.clone(),
                    initialization.clone(),
                    pool.clone(),
                    http_client.clone(),
                    models_dir.clone(),
                );
            }
            return;
        }

        let Some(parent_dir) = gguf_path.parent() else {
            let message = format!(
                "GGUF path has no parent directory: '{}'",
                gguf_path.display()
            );
            error!("{message}");
            let restart = finish_llm_initialization_with_progress(
                &app,
                &initialization,
                generation,
                model_download::ModelDownloadProgress {
                    model_id: model_id.to_string(),
                    current_file: String::new(),
                    file_downloaded: 0,
                    file_total: 0,
                    files_completed: 0,
                    total_files: 0,
                    stage: model_download::DownloadStage::Error { message },
                },
            )
            .await;
            if restart {
                schedule_llm_initialization_with_resources(
                    app.clone(),
                    llm_state.clone(),
                    initialization.clone(),
                    pool.clone(),
                    http_client.clone(),
                    models_dir.clone(),
                );
            }
            return;
        };

        info!("model downloaded, building mistral.rs pipeline...");
        let builder = mistralrs::GgufModelBuilder::new(
            parent_dir.to_string_lossy().to_string(),
            vec![gguf_filename],
        )
        .with_tok_model_id(tokenizer_model);

        match builder.build().await {
            Ok(model) => {
                let current_mode = app_settings
                    .get("inference_mode")
                    .await
                    .unwrap_or_default()
                    .unwrap_or_else(|| "local".to_string());
                if current_mode != "local" {
                    let restart = finish_llm_initialization(&initialization, generation).await;
                    if restart {
                        schedule_llm_initialization_with_resources(
                            app.clone(),
                            llm_state.clone(),
                            initialization.clone(),
                            pool.clone(),
                            http_client.clone(),
                            models_dir.clone(),
                        );
                    }
                    return;
                }

                let restart = finish_local_model_initialization(
                    &app,
                    &llm_state,
                    &initialization,
                    generation,
                    model,
                )
                .await;
                if restart {
                    schedule_llm_initialization_with_resources(
                        app.clone(),
                        llm_state.clone(),
                        initialization.clone(),
                        pool.clone(),
                        http_client.clone(),
                        models_dir.clone(),
                    );
                }
            }
            Err(error) => {
                let message = format!("Failed to initialize local model: {error}");
                error!("{message}");
                let restart = finish_llm_initialization_with_progress(
                    &app,
                    &initialization,
                    generation,
                    model_download::ModelDownloadProgress {
                        model_id,
                        current_file: String::new(),
                        file_downloaded: 0,
                        file_total: 0,
                        files_completed: 0,
                        total_files: 0,
                        stage: model_download::DownloadStage::Error { message },
                    },
                )
                .await;
                if restart {
                    schedule_llm_initialization_with_resources(
                        app.clone(),
                        llm_state.clone(),
                        initialization.clone(),
                        pool.clone(),
                        http_client.clone(),
                        models_dir.clone(),
                    );
                }
            }
        }
    });
}

#[tauri::command]
async fn begin_first_run_init(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
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

    let models_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("models");

    if !embedder_ready && !EMBEDDER_INIT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        let embedder_state = state.embedder.clone();
        let embedder_handle = app.clone();
        let models_dir_embedder = models_dir.clone();
        let embedder_pool = state.db_pool.clone();
        let embedder_job_queue = state.job_queue.clone();
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

                    // Recovery runs after the embedder is ready. Awaiting sends applies
                    // producer backpressure instead of silently dropping recovered work.
                    let repo = persistence::DocumentRepository::new(embedder_pool.clone());
                    let recovered_ids = {
                        let _document_operation = document_operations.write().await;
                        let mut recovered_ids = HashSet::new();
                        let settings = AppSettingsRepository::new(embedder_pool.clone());
                        match desired_indexing_provenance(&settings).await {
                            Ok(desired_provenance) => match repo
                                .requeue_indexed_with_stale_provenance(&desired_provenance)
                                .await
                            {
                                Ok(ids) => recovered_ids.extend(ids),
                                Err(e) => {
                                    error!(
                                        "startup recovery — failed to requeue stale indexed documents: {e}"
                                    );
                                }
                            },
                            Err(e) => {
                                error!(
                                    "startup recovery — failed to read desired indexing provenance: {e}"
                                );
                            }
                        }

                        match repo.reset_embedding_to_queued().await {
                            Ok(ids) => recovered_ids.extend(ids),
                            Err(e) => {
                                error!(
                                    "startup recovery — failed to reset embedding documents: {e}"
                                )
                            }
                        }
                        match repo.reset_failed_to_queued(3).await {
                            Ok(ids) => recovered_ids.extend(ids),
                            Err(e) => {
                                error!(
                                    "startup recovery — failed to reset retryable documents: {e}"
                                )
                            }
                        }
                        match repo.get_queued_ids().await {
                            Ok(ids) => recovered_ids.extend(ids),
                            Err(e) => {
                                error!("startup recovery — failed to list queued documents: {e}")
                            }
                        }
                        recovered_ids
                    };

                    let recovered_count = recovered_ids.len();
                    for id in recovered_ids {
                        if let Err(e) = embedder_job_queue.send(IngestionJob::background(id)).await
                        {
                            error!(
                                "startup recovery — document worker stopped before recovery completed: {e}"
                            );
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
async fn retry_local_model_initialization(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let mode = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    if mode != "local" {
        return Err(AppError::InvalidInput(
            "Local model retry is unavailable while external inference is selected".into(),
        ));
    }
    if state.local_llm.read().await.is_some() {
        let _ = app.emit("llm_ready", ());
        return Ok(());
    }

    let models_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("models");
    request_llm_initialization(&app, &state, models_dir).await;
    Ok(())
}

#[tauri::command]
async fn upload_document(
    app: tauri::AppHandle,
    filename: String,
    file_bytes: Vec<u8>,
    channel: tauri::ipc::Channel<IngestionProgress>,
) -> Result<String, AppError> {
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
        return Err(AppError::DuplicateDocument(format!(
            "This file was already uploaded as '{}' (status: {:?})",
            existing.filename, existing.status
        )));
    }

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.pdf", &doc_id);
    let storage_path = docs_dir.join(&storage_filename);
    let relative_storage_path = format!("documents/{}", storage_filename);

    tokio::fs::write(&storage_path, &file_bytes).await?;

    let created_at = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string();

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
        indexing_provenance: None,
        error_message: None,
        retry_count: 0,
    };

    if let Err(e) = repo.insert(&doc).await {
        let _ = tokio::fs::remove_file(&storage_path).await;
        return Err(e);
    }

    if state
        .job_queue
        .try_send(IngestionJob {
            document_id: doc_id.clone(),
            progress_channel: Some(channel),
        })
        .is_err()
    {
        if let Err(cleanup_error) = repo.delete_by_id(&doc_id).await {
            warn!(
                "queue-full upload rollback could not delete document {doc_id}; scheduling durable enqueue: {cleanup_error}"
            );
            let queue = state.job_queue.clone();
            let queued_document_id = doc_id.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(send_error) = queue
                    .send(IngestionJob::background(queued_document_id))
                    .await
                {
                    error!("queue-full upload recovery producer stopped: {send_error}");
                }
            });
            return Err(AppError::QueueFull);
        }
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
    let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);
    state
        .active_queries
        .lock()
        .map_err(|_| AppError::Internal("Query cancellation state is poisoned".into()))?
        .insert(request_id.clone(), cancel_sender);
    let _active_query = ActiveQuery {
        queries: state.active_queries.clone(),
        request_id: request_id.clone(),
    };
    let settings_repo = AppSettingsRepository::new(state.db_pool.clone());

    let embedder_opt = {
        let guard = state.embedder.read().await;
        guard.clone()
    };
    let embedder = embedder_opt
        .ok_or_else(|| AppError::Internal("Embedding model not yet loaded. Please wait.".into()))?;
    let vector_db = state.vector_db.clone();
    let local_llm = state.local_llm.clone();

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

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
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

            citations.push(Citation {
                document_id: result.document_id,
                filename: doc.filename,
                storage_path: app_local_data
                    .join(&doc.storage_path)
                    .to_string_lossy()
                    .to_string(),
                page: result.page as i32,
                chunk_content: result.content,
                score: 1.0 / (1.0 + result.score),
                is_hybrid,
            });
        }
        citations.truncate(RETRIEVAL_RESULT_LIMIT);
        citations
    };
    require_retrieved_citations(&citations)?;

    let context = rag_pipeline::build_rag_prompt(&query, &citations);
    let system_prompt = "You are a helpful assistant that answers questions using only the provided document excerpts. Answer concisely. The structured source list accompanying the answer is the only citation authority; do not include page numbers, citation markers, or source references in the answer. Do not use your own knowledge. Do not use <|think|> or </|think|> tags.";

    let mode = settings_repo
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    let provider = if mode == "external" {
        let config = external_inference_config(&settings_repo).await?;
        let http_client = state.http_client.clone();
        probe_external_inference_readiness(&http_client, &config).await?;
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
async fn list_documents(state: tauri::State<'_, AppState>) -> Result<Vec<Document>, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    repo.list_all().await
}

fn document_pdf_path(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let stored_path = std::path::Path::new(storage_path);
    if stored_path.is_absolute()
        || stored_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::Internal(
            "Document has an unsafe storage path".into(),
        ));
    }

    Ok(app_local_data.join(stored_path))
}

async fn finish_document_deletion(
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    app_local_data: &std::path::Path,
    document: &Document,
) -> Result<(), AppError> {
    vector_db.delete_chunks(&document.id).await?;

    let pdf_path = document_pdf_path(app_local_data, &document.storage_path)?;
    match tokio::fs::remove_file(pdf_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(AppError::Io(error.to_string())),
    }

    repo.delete_by_id(&document.id).await
}

async fn reconcile_pending_deletions(
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    app_local_data: &std::path::Path,
) {
    let documents = match repo.get_deleting_documents().await {
        Ok(documents) => documents,
        Err(error) => {
            error!("startup deletion recovery — failed to list deleting documents: {error}");
            return;
        }
    };

    for document in documents {
        let document_id = document.id.clone();
        if let Err(error) =
            finish_document_deletion(repo, vector_db, app_local_data, &document).await
        {
            error!("startup deletion recovery — failed to delete document {document_id}: {error}");
        } else {
            info!("startup deletion recovery — deleted document {document_id}");
        }
    }
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
    if let Some(document) = document {
        let app_local_data = app
            .path()
            .app_local_data_dir()
            .map_err(|error| AppError::Io(error.to_string()))?;
        document_pdf_path(&app_local_data, &document.storage_path)?;
        repo.mark_deleting(&id).await?;
        finish_document_deletion(&repo, &state.vector_db, &app_local_data, &document).await?;
    } else {
        state.vector_db.delete_chunks(&id).await?;
    }
    let _ = app.emit(
        "document_deleted",
        serde_json::json!({
            "document_id": &id,
        }),
    );

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
async fn get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<HashMap<String, String>, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let mut settings = HashMap::new();
    let keys = vec![
        "inference_mode",
        "external_base_url",
        "external_model",
        "generation_model",
        "tokenizer_model",
        "gguf_filename",
        "use_query_enhancement",
        "chunk_size",
        "chunk_overlap",
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
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    validate_setting(&key, &value)?;
    if matches!(
        key.as_str(),
        "generation_model" | "tokenizer_model" | "gguf_filename"
    ) {
        set_local_model_setting_and_restart_if_active(&app, &state, &key, &value).await?;
        return Ok(());
    }

    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let requeued_ids = if matches!(key.as_str(), "chunk_size" | "chunk_overlap") {
        {
            let _document_operation = state.document_operations.write().await;
            let mut desired_provenance = desired_indexing_provenance(&repo).await?;
            match key.as_str() {
                "chunk_size" => {
                    desired_provenance.chunk_size = parse_chunk_setting("chunk_size", &value)?;
                }
                "chunk_overlap" => {
                    desired_provenance.chunk_overlap =
                        parse_chunk_setting("chunk_overlap", &value)?;
                }
                _ => unreachable!("chunk settings branch only accepts chunk settings"),
            }
            validate_indexing_provenance(&desired_provenance)?;
            repo.set_indexing_setting_and_requeue_stale_documents(&key, &value, &desired_provenance)
                .await?
        }
    } else {
        repo.set(&key, &value).await?;
        Vec::new()
    };

    enqueue_requeued_documents(state.job_queue.clone(), requeued_ids)?;
    if key == "inference_mode" {
        if value == "external" {
            restart_external_readiness_probe(&app, &state, true).await?;
        } else {
            restart_local_model_initialization_if_active(&app, &state).await?;
        }
    } else if matches!(key.as_str(), "external_base_url" | "external_model") {
        revalidate_external_readiness_if_active(&app, &state).await?;
    }
    Ok(())
}

#[tauri::command]
async fn set_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<bool, AppError> {
    let saved = tokio::task::spawn_blocking(move || {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| AppError::Internal(format!("keyring: {e}")))?;
        entry
            .set_password(&key)
            .map_err(|e| AppError::Internal(format!("keyring set: {e}")))?;
        Ok::<bool, AppError>(true)
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))??;
    revalidate_external_readiness_if_active(&app, &state).await?;
    Ok(saved)
}

#[tauri::command]
async fn has_api_key() -> Result<bool, AppError> {
    tokio::task::spawn_blocking(
        || match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => match entry.get_password() {
                Ok(pwd) => Ok(!pwd.is_empty()),
                Err(_) => Ok(false),
            },
            Err(_) => Ok(false),
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))?
}

#[tauri::command]
async fn delete_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, AppError> {
    let deleted = tokio::task::spawn_blocking(|| {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| AppError::Internal(format!("keyring: {e}")))?;
        entry
            .delete_credential()
            .map_err(|e| AppError::Internal(format!("keyring delete: {e}")))?;
        Ok::<bool, AppError>(true)
    })
    .await
    .map_err(|e| AppError::Internal(format!("keyring task panicked: {e}")))??;
    revalidate_external_readiness_if_active(&app, &state).await?;
    Ok(deleted)
}

#[tauri::command]
async fn get_storage_info(
    app: tauri::AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
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
async fn vacuum_database(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    sqlx::query("VACUUM")
        .execute(&state.db_pool)
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(serde_json::json!({"status": "vacuumed"}))
}

fn remove_path_if_present(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn remove_non_fastembed_models(models_dir: &std::path::Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(models_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().contains("fastembed") {
            remove_path_if_present(&entry.path())?;
        }
    }
    Ok(())
}

fn complete_pending_reset(
    app_local_data: &std::path::Path,
    sentinel: &std::path::Path,
) -> Result<(), AppError> {
    let contents = match std::fs::read_to_string(sentinel) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not read pending reset marker: {error}"
            )));
        }
    };
    let config: serde_json::Value = serde_json::from_str(&contents).map_err(|error| {
        AppError::InvalidInput(format!("Pending reset marker is invalid: {error}"))
    })?;
    let delete_models = config["delete_models"].as_bool().unwrap_or(false);

    let mut failures = Vec::new();
    for path in [
        app_local_data.join("documents"),
        app_local_data.join(".lancedb"),
        app_local_data.join("app.db"),
        app_local_data.join("app.db-wal"),
        app_local_data.join("app.db-shm"),
    ] {
        if let Err(error) = remove_path_if_present(&path) {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    if delete_models {
        let models_dir = app_local_data.join("models");
        if let Err(error) = remove_non_fastembed_models(&models_dir) {
            failures.push(format!("{}: {error}", models_dir.display()));
        }
    }

    if !failures.is_empty() {
        return Err(AppError::Io(format!(
            "Pending data reset incomplete: {}",
            failures.join("; "),
        )));
    }

    std::fs::remove_file(sentinel)
        .map_err(|error| AppError::Io(format!("Could not clear pending reset marker: {error}")))?;
    Ok(())
}

#[tauri::command]
async fn reset_all_data(
    app: tauri::AppHandle,
    delete_models: bool,
) -> Result<serde_json::Value, AppError> {
    let app_local_data = app
        .path()
        .app_local_data_dir()
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
        .setup(|app| {
            let app_local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            // Complete a requested reset before opening either data store. On any
            // failure the sentinel remains in place so the next launch retries it.
            let sentinel = app_local_data.join(".reset_pending");
            if let Err(error) = complete_pending_reset(&app_local_data, &sentinel) {
                error!("pending data reset failed: {error}");
                return Err(Box::new(std::io::Error::other(error.to_string())));
            }


            let pool = tauri::async_runtime::block_on(init_pool(&app_local_data))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;

            let embedder: Arc<tokio::sync::RwLock<Option<Arc<std::sync::Mutex<fastembed::TextEmbedding>>>>> =
                Arc::new(tokio::sync::RwLock::new(None));

            let (tx, mut rx) = mpsc::channel::<IngestionJob>(32);

            let lancedb_dir = app_local_data.join(".lancedb");
            let vector_db = tauri::async_runtime::block_on(VectorDb::init(&lancedb_dir))
                .map_err(|e| Box::new(std::io::Error::other(
                    e.to_string(),
                )))?;
            let vector_db = Arc::new(vector_db);
            let deletion_repo = DocumentRepository::new(pool.clone());
            tauri::async_runtime::block_on(reconcile_pending_deletions(
                &deletion_repo,
                &vector_db,
                &app_local_data,
            ));

            let local_llm = Arc::new(tokio::sync::RwLock::new(None));
            let llm_initialization = Arc::new(tokio::sync::Mutex::new(
                state::LlmInitializationState::default(),
            ));
            let registry_read_retries = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let document_operations = Arc::new(tokio::sync::RwLock::new(()));
            let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx.clone(),
                embedder: embedder.clone(),
                vector_db: vector_db.clone(),
                local_llm: local_llm.clone(),
                llm_initialization,
                registry_read_retries: registry_read_retries.clone(),
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
            let job_queue_worker = tx;
            let registry_read_retries_worker = registry_read_retries;
            tauri::async_runtime::spawn(async move {
                let repo = DocumentRepository::new(pool.clone());
                while let Some(job) = rx.recv().await {
                    let IngestionJob {
                        document_id: doc_id,
                        progress_channel,
                    } = job;
                    let retry_progress_channel = progress_channel.clone();
                    let progress_callback: Arc<dyn Fn(IngestionProgress) + Send + Sync> =
                        match progress_channel.map(start_ingestion_progress_forwarder) {
                            Some(sender) => Arc::new(move |progress| {
                                let _ = sender.send(progress);
                            }),
                            None => Arc::new(|_| {}),
                        };
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
                        Ok(Some(doc)) if doc.status == DocumentStatus::Queued => {
                            registry_read_retries_worker.lock().await.remove(&doc_id);
                            let settings_repo = AppSettingsRepository::new(pool.clone());
                            match desired_indexing_provenance(&settings_repo).await {
                                Ok(provenance) => {
                                    let pdf_path = docs_dir.join(&doc.storage_path);
                                    let app_for_stage = app_handle.clone();
                                    let doc_id_for_stage = doc_id.clone();
                                    rag_pipeline::process_document(
                                        &pdf_path,
                                        &doc_id,
                                        provenance,
                                        embedder,
                                        &repo,
                                        &vector_db,
                                        move |stage| {
                                            let _ = app_for_stage.emit("ingestion_progress", serde_json::json!({
                                                "document_id": &doc_id_for_stage,
                                                "stage": stage,
                                            }));
                                        },
                                        Arc::clone(&progress_callback),
                                    )
                                    .await
                                }
                                Err(error) => Err(error),
                            }
                        }
                        Ok(Some(document)) => {
                            registry_read_retries_worker.lock().await.remove(&doc_id);
                            debug!(
                                "document {doc_id} is {:?}; ignoring stale queue entry",
                                document.status,
                            );
                            continue;
                        }
                        Ok(None) => {
                            registry_read_retries_worker.lock().await.remove(&doc_id);
                            warn!("document {doc_id} not found in DB — may have been deleted");
                            continue;
                        }
                        Err(error) => {
                            let attempt = {
                                let mut retries = registry_read_retries_worker.lock().await;
                                let attempt = retries.entry(doc_id.clone()).or_insert(0);
                                *attempt = attempt.saturating_add(1);
                                *attempt
                            };
                            let delay = registry_read_retry_delay(attempt);
                            warn!(
                                "database error while fetching document {doc_id}: {error}; retrying in {delay:?}"
                            );
                            let queue = job_queue_worker.clone();
                            let retry_document_id = doc_id.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(delay).await;
                                if let Err(error) = queue
                                    .send(IngestionJob {
                                        document_id: retry_document_id,
                                        progress_channel: retry_progress_channel,
                                    })
                                    .await
                                {
                                    error!("registry read retry producer stopped: {error}");
                                }
                            });
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
                        Err(error) => {
                            let message = error.to_string();
                            error!("document {doc_id} processing failed: {message}");
                            let _ = repo.mark_failed(&doc_id, &message).await;
                            progress_callback(IngestionProgress::Error {
                                document_id: doc_id.clone(),
                                message: message.clone(),
                            });
                            let _ = app_handle.emit("document_failed", serde_json::json!({
                                "document_id": &doc_id,
                                "error": message,
                            }));
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            begin_first_run_init,
            retry_local_model_initialization,
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
    use super::{
        ExternalInferenceConfig, ExternalModelsResponse, IngestionJob, enqueue_requeued_documents,
        external_model_is_available, finish_llm_initialization, invalidate_llm_initialization,
        llm_initialization_is_current, normalize_external_base_url, registry_read_retry_delay,
        remove_non_fastembed_models, require_retrieved_citations, sse_event_boundary,
        sse_event_data, validate_setting, validate_upload,
    };
    use domain::AppError;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{Mutex, mpsc};
    use tokio::time::timeout;

    #[test]
    fn validates_external_model() {
        assert!(validate_setting("external_model", "gpt-4o-mini").is_ok());
        assert!(validate_setting("external_model", "").is_err());
        assert!(validate_setting("external_model", "   ").is_err());
    }

    #[test]
    fn normalizes_external_endpoints_without_dropping_base_path() {
        let config = ExternalInferenceConfig {
            base_url: normalize_external_base_url("https://example.test/v1/").unwrap(),
            model: "gpt-4o-mini".into(),
            api_key: None,
        };
        let trailing = ExternalInferenceConfig {
            base_url: normalize_external_base_url("https://example.test/v1///").unwrap(),
            model: "gpt-4o-mini".into(),
            api_key: None,
        };

        assert_eq!(
            config.models_url().unwrap().as_str(),
            "https://example.test/v1/models"
        );
        assert_eq!(
            config.chat_completions_url().unwrap().as_str(),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(
            trailing.models_url().unwrap().as_str(),
            "https://example.test/v1/models"
        );
        assert!(normalize_external_base_url("https://user@example.test/v1").is_err());
        assert!(normalize_external_base_url("https://example.test/v1?tenant=a").is_err());
    }
    #[test]
    fn requires_configured_external_model_in_models_response() {
        let models: ExternalModelsResponse =
            serde_json::from_str(r#"{"data":[{"id":"gpt-4o-mini"},{"id":"other-model"}]}"#)
                .unwrap();

        assert!(external_model_is_available(&models, "gpt-4o-mini"));
        assert!(!external_model_is_available(&models, "missing-model"));
    }

    #[test]
    fn validates_tokenizer_model() {
        assert!(validate_setting("tokenizer_model", "Qwen/Qwen3-1.7B").is_ok());
        for invalid in [
            "",
            "Qwen",
            "Qwen/",
            "/Qwen3-1.7B",
            "Qwen/Qwen3/1.7B",
            "Qwen /Qwen3-1.7B",
        ] {
            assert!(validate_setting("tokenizer_model", invalid).is_err());
        }
    }

    #[test]
    fn refuses_queries_without_retrieved_citations() {
        let error = require_retrieved_citations(&[]).unwrap_err();
        assert!(matches!(&error, domain::AppError::Inference(_)));
        assert_eq!(
            serde_json::to_value(error).unwrap()["kind"],
            serde_json::Value::String("Inference".into())
        );
    }

    #[test]
    fn accepts_utf8_pdf_filename_in_typed_upload() {
        assert!(validate_upload("操作说明.pdf", b"%PDF-").is_ok());
    }
    #[test]
    fn removes_non_fastembed_model_entries() {
        let models_dir =
            std::env::temp_dir().join(format!("ragtag-model-cleanup-{}", uuid::Uuid::now_v7()));
        let other_model = models_dir.join("generation-model");
        let embedder_models = models_dir.join("fastembed");
        std::fs::create_dir_all(&other_model).unwrap();
        std::fs::create_dir_all(&embedder_models).unwrap();
        std::fs::write(other_model.join("model.gguf"), "model").unwrap();
        std::fs::write(embedder_models.join("model.onnx"), "embedder").unwrap();

        remove_non_fastembed_models(&models_dir).unwrap();

        assert!(!other_model.exists());
        assert!(embedder_models.exists());
        std::fs::remove_dir_all(models_dir).unwrap();
    }

    #[test]
    fn accepts_pdf_signature_in_short_payload() {
        assert!(validate_upload("minimal.pdf", b"%PDF-").is_ok());
    }

    #[tokio::test]
    async fn reindex_queue_overflow_returns_without_blocking_and_recovers_in_background() {
        let (queue, mut receiver) = mpsc::channel(1);
        queue
            .try_send(IngestionJob::background("already-queued".to_string()))
            .unwrap();

        let error = enqueue_requeued_documents(
            queue.clone(),
            vec!["first-stale".to_string(), "second-stale".to_string()],
        )
        .unwrap_err();
        assert!(matches!(error, AppError::QueueFull));
        assert_eq!(
            receiver.recv().await.map(|job| job.document_id).as_deref(),
            Some("already-queued")
        );
        assert_eq!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .map(|job| job.document_id)
                .as_deref(),
            Some("first-stale")
        );
        assert_eq!(
            timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .map(|job| job.document_id)
                .as_deref(),
            Some("second-stale")
        );
    }

    #[tokio::test]
    async fn local_model_config_invalidations_coalesce_to_the_latest_generation() {
        let initialization = Arc::new(Mutex::new(super::state::LlmInitializationState::default()));
        let first_generation = {
            let mut current = initialization.lock().await;
            let generation = current.generation;
            current.in_progress = true;
            current.in_progress_generation = Some(generation);
            generation
        };

        {
            let mut current = initialization.lock().await;
            invalidate_llm_initialization(&mut current);
            current.restart_requested = true;
            invalidate_llm_initialization(&mut current);
            current.restart_requested = true;
            assert_eq!(current.generation, first_generation.wrapping_add(2));
            assert_eq!(current.in_progress_generation, Some(first_generation));
        }

        assert!(!llm_initialization_is_current(&initialization, first_generation).await);
        assert!(finish_llm_initialization(&initialization, first_generation).await);
        assert!(!finish_llm_initialization(&initialization, first_generation).await);

        let latest_generation = {
            let mut current = initialization.lock().await;
            assert!(!current.in_progress);
            assert_eq!(current.in_progress_generation, None);
            let generation = current.generation;
            current.in_progress = true;
            current.in_progress_generation = Some(generation);
            generation
        };
        assert!(llm_initialization_is_current(&initialization, latest_generation).await);
        assert!(!finish_llm_initialization(&initialization, latest_generation).await);
    }

    #[test]
    fn decodes_sse_only_after_complete_utf8_event() {
        let mut buffer = b"data: {\"choices\":[{\"delta\":{\"content\":\"caf\xc3".to_vec();
        assert!(sse_event_boundary(&buffer).is_none());
        buffer.extend_from_slice(b"\xa9\"}}]}\n\n");

        let (event_end, _) = sse_event_boundary(&buffer).unwrap();
        let event = std::str::from_utf8(&buffer[..event_end]).unwrap();
        assert_eq!(
            sse_event_data(event),
            Some("{\"choices\":[{\"delta\":{\"content\":\"café\"}}]}"),
        );
    }

    #[test]
    fn caps_registry_read_retry_delay() {
        assert_eq!(registry_read_retry_delay(1), Duration::from_millis(250));
        assert_eq!(registry_read_retry_delay(3), Duration::from_secs(1));
        assert_eq!(registry_read_retry_delay(u32::MAX), Duration::from_secs(30));
    }
}
