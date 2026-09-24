mod state;
pub use state::{AppState, IngestionJob};
mod model_download;

use domain::{
    AppError, Citation, Document, DocumentPreview, DocumentStatus, IngestionProgress, TokenEvent,
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
const EXTERNAL_REQUEST_START_TIMEOUT: Duration = Duration::from_secs(15);
const EXTERNAL_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_EXTERNAL_QUERY_BYTES: usize = 32 * 1024;
const MAX_EXTERNAL_REQUEST_ID_BYTES: usize = 64;
const MAX_EXTERNAL_MODELS_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_EXTERNAL_MODELS: usize = 512;
const MAX_EXTERNAL_MODEL_ID_BYTES: usize = 256;
const MAX_SSE_EVENT_BYTES: usize = 64 * 1024;
const MAX_SSE_TOKEN_BYTES: usize = 16 * 1024;
const MAX_SSE_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_EXTERNAL_BASE_URL: &str = "http://localhost:11434/v1";
const MAX_AUTOMATIC_RECOVERY_ATTEMPTS: i32 = 3;
const MAX_SSE_INCOMPLETE_EVENT_BYTES: usize = MAX_SSE_EVENT_BYTES;

fn validate_upload(filename: &str, bytes: &[u8]) -> Result<&'static str, AppError> {
    let name = std::path::Path::new(filename);
    if filename.is_empty()
        || filename.len() > 255
        || name.file_name().and_then(|name| name.to_str()) != Some(filename)
    {
        return Err(AppError::InvalidInput(
            "Upload filename must be a single file name no longer than 255 characters".into(),
        ));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::InvalidInput(
            "Document exceeds the 50 MiB upload limit".into(),
        ));
    }

    match name
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => {
            let prefix = &bytes[..bytes.len().min(1024)];
            if !prefix.windows(5).any(|window| window == b"%PDF-") {
                return Err(AppError::InvalidInput("Upload is not a PDF".into()));
            }
            Ok("pdf")
        }
        Some("docx") => {
            if !bytes.starts_with(b"PK\x03\x04") {
                return Err(AppError::InvalidInput(
                    "Upload is not a DOCX archive".into(),
                ));
            }
            Ok("docx")
        }
        _ => {
            if bytes.len() > rag_pipeline::extractor::EXTRACTED_TEXT_MAX_BYTES {
                return Err(AppError::InvalidInput(format!(
                    "Text document exceeds the {} MiB indexing limit",
                    rag_pipeline::extractor::EXTRACTED_TEXT_MAX_BYTES / (1024 * 1024)
                )));
            }
            if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
                return Err(AppError::InvalidInput(
                    "Ragtag accepts UTF-8 text files, PDF documents, and DOCX documents".into(),
                ));
            }
            Ok("text")
        }
    }
}

fn storage_extension(filename: &str, file_type: &str) -> String {
    std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| file_type.to_string())
}

fn is_managed_document_source_filename(filename: &str) -> bool {
    let Some((id, extension)) = filename.split_once('.') else {
        return false;
    };

    let is_canonical_v7_uuid = uuid::Uuid::parse_str(id)
        .is_ok_and(|uuid| uuid.get_version_num() == 7 && uuid.to_string() == id);

    is_canonical_v7_uuid
        && (1..=16).contains(&extension.len())
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_managed_document_temporary_filename(filename: &str) -> bool {
    let Some(filename) = filename
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((source_filename, temporary_id)) = filename.rsplit_once('.') else {
        return false;
    };
    is_managed_document_source_filename(source_filename)
        && uuid::Uuid::parse_str(temporary_id)
            .is_ok_and(|uuid| uuid.get_version_num() == 7 && uuid.to_string() == temporary_id)
}

fn validate_query_request_id(request_id: &str) -> Result<(), AppError> {
    if request_id.len() > MAX_EXTERNAL_REQUEST_ID_BYTES
        || uuid::Uuid::parse_str(request_id).is_err()
    {
        return Err(AppError::InvalidInput(
            "Query request ID must be a UUID no longer than 64 bytes".into(),
        ));
    }

    Ok(())
}

fn validate_query_request(query: &str, request_id: &str) -> Result<(), AppError> {
    if query.len() > MAX_EXTERNAL_QUERY_BYTES {
        return Err(AppError::InvalidInput(
            "Query exceeds the 32 KiB limit".into(),
        ));
    }
    validate_query_request_id(request_id)
}

fn inference_mode_is_unchanged(current_mode: Option<&str>, requested_mode: &str) -> bool {
    current_mode.unwrap_or("local") == requested_mode
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

fn sse_incomplete_buffer_exceeds_limit(buffer: &[u8]) -> bool {
    buffer.len() > MAX_SSE_INCOMPLETE_EVENT_BYTES
}

fn sse_complete_event_exceeds_limit(event_bytes: usize) -> bool {
    event_bytes > MAX_SSE_EVENT_BYTES
}

fn sse_token_exceeds_limit(token: &str) -> bool {
    token.len() > MAX_SSE_TOKEN_BYTES
}

fn sse_output_exceeds_limit(total_output_bytes: usize, token: &str) -> bool {
    total_output_bytes.saturating_add(token.len()) > MAX_SSE_OUTPUT_BYTES
}

fn sse_event_data(event: &str) -> Result<Option<String>, AppError> {
    let mut data_lines = Vec::new();
    let mut saw_data = false;

    for line in event.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        if field == "data" {
            saw_data = true;
            data_lines.push(value);
        }
    }

    Ok(saw_data.then(|| data_lines.join("\n")))
}

fn external_sse_token(data: &str) -> Result<Option<String>, AppError> {
    let payload: serde_json::Value = serde_json::from_str(data).map_err(|_| {
        AppError::Inference("External inference sent malformed SSE JSON payload.".into())
    })?;
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("External provider returned an error.");
        return Err(AppError::Inference(format!("External provider error: {message}")));
    }
    let choices = payload
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            AppError::Inference("External inference sent an SSE payload without choices.".into())
        })?;
    let Some(choice) = choices.first().and_then(serde_json::Value::as_object) else {
        return Ok(None);
    };
    let delta = choice
        .get("delta")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            AppError::Inference(
                "External inference sent an SSE payload with an invalid delta.".into(),
            )
        })?;

    match delta.get("content") {
        Some(serde_json::Value::String(token)) => Ok((!token.is_empty()).then(|| token.clone())),
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(_) => Err(AppError::Inference(
            "External inference sent an SSE payload with a non-text token.".into(),
        )),
    }
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

fn parse_external_models_response(bytes: &[u8]) -> Result<ExternalModelsResponse, AppError> {
    let models = serde_json::from_slice::<ExternalModelsResponse>(bytes).map_err(|error| {
        AppError::Inference(format!(
            "External inference readiness check received an invalid /models response: {error}"
        ))
    })?;
    if models.data.len() > MAX_EXTERNAL_MODELS {
        return Err(AppError::Inference(format!(
            "External inference /models response exceeds the {MAX_EXTERNAL_MODELS} model limit"
        )));
    }
    if models
        .data
        .iter()
        .any(|model| model.id.len() > MAX_EXTERNAL_MODEL_ID_BYTES)
    {
        return Err(AppError::Inference(format!(
            "External inference /models response contains a model ID longer than {MAX_EXTERNAL_MODEL_ID_BYTES} bytes"
        )));
    }

    Ok(models)
}

fn external_models_response_too_large(response: &reqwest::Response) -> bool {
    response
        .content_length()
        .is_some_and(|length| length > MAX_EXTERNAL_MODELS_RESPONSE_BYTES as u64)
}

async fn read_external_models_response(
    response: reqwest::Response,
) -> Result<ExternalModelsResponse, AppError> {
    use futures_util::StreamExt;

    if external_models_response_too_large(&response) {
        return Err(AppError::Inference(format!(
            "External inference /models response exceeds the {} byte limit",
            MAX_EXTERNAL_MODELS_RESPONSE_BYTES
        )));
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next())
        .await
        .map_err(|_| AppError::Inference("External inference /models response timed out".into()))?
    {
        let chunk = chunk.map_err(|error| {
            AppError::Inference(format!(
                "External inference readiness check received an invalid /models response: {error}"
            ))
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_EXTERNAL_MODELS_RESPONSE_BYTES {
            return Err(AppError::Inference(format!(
                "External inference /models response exceeds the {} byte limit",
                MAX_EXTERNAL_MODELS_RESPONSE_BYTES
            )));
        }
        bytes.extend_from_slice(&chunk);
    }

    parse_external_models_response(&bytes)
}

async fn read_external_models_response_cancellable(
    response: reqwest::Response,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Option<ExternalModelsResponse>, AppError> {
    use futures_util::StreamExt;

    if external_models_response_too_large(&response) {
        return Err(AppError::Inference(format!(
            "External inference /models response exceeds the {} byte limit",
            MAX_EXTERNAL_MODELS_RESPONSE_BYTES
        )));
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.changed() => return Ok(None),
            result = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next()) => result,
        }
        .map_err(|_| AppError::Inference("External inference /models response timed out".into()))?;
        let Some(chunk) = chunk else {
            return parse_external_models_response(&bytes).map(Some);
        };
        let chunk = chunk.map_err(|error| {
            AppError::Inference(format!(
                "External inference readiness check received an invalid /models response: {error}"
            ))
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_EXTERNAL_MODELS_RESPONSE_BYTES {
            return Err(AppError::Inference(format!(
                "External inference /models response exceeds the {} byte limit",
                MAX_EXTERNAL_MODELS_RESPONSE_BYTES
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
}

#[derive(serde::Serialize)]
struct ExternalProviderConnection {
    models: Vec<String>,
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

async fn read_external_api_key(
    settings: &AppSettingsRepository,
) -> Result<Option<String>, AppError> {
    let from_keychain = tokio::task::spawn_blocking(|| {
        let entry = match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("System keychain entry unavailable: {error}");
                return None;
            }
        };
        match entry.get_password() {
            Ok(api_key) if !api_key.is_empty() => Some(api_key),
            Ok(_) | Err(keyring::v1::Error::NoEntry) => None,
            Err(error) => {
                log::warn!("System keychain password read failed: {error}");
                None
            }
        }
    })
    .await
    .unwrap_or(None);

    if let Some(key) = from_keychain {
        return Ok(Some(key));
    }

    let from_db = settings
        .get("external_api_key")
        .await?
        .filter(|key| !key.trim().is_empty());

    Ok(from_db)
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
        api_key: read_external_api_key(settings).await?,
    })
}

fn external_model_is_available(models: &ExternalModelsResponse, configured_model: &str) -> bool {
    let configured_model = configured_model.trim();
    models
        .data
        .iter()
        .any(|available_model| available_model.id.trim() == configured_model)
}

fn external_model_ids(models: &ExternalModelsResponse) -> Vec<String> {
    let mut ids = models
        .data
        .iter()
        .map(|model| model.id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

async fn fetch_external_models(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
) -> Result<ExternalModelsResponse, AppError> {
    let models_url = config.models_url()?;
    let request = apply_external_auth(client.get(models_url), config.api_key.as_deref());
    let response = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send())
        .await
        .map_err(|_| AppError::Inference("External inference readiness check timed out".into()))?
        .map_err(|error| {
            AppError::Inference(format!(
                "External inference readiness check failed: {error}"
            ))
        })?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        return Err(AppError::Inference(format!(
            "Authentication required: endpoint returned HTTP {}",
            response.status()
        )));
    } else if !response.status().is_success() {
        return Err(AppError::Inference(format!(
            "External inference readiness check failed: endpoint returned HTTP {}",
            response.status()
        )));
    }

    read_external_models_response(response).await
}

async fn probe_external_inference_readiness(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
) -> Result<(), AppError> {
    let models = fetch_external_models(client, config).await?;
    if !external_model_is_available(&models, &config.model) {
        return Err(AppError::Inference(format!(
            "External inference model '{}' is not available from the configured endpoint",
            config.model
        )));
    }

    Ok(())
}

async fn probe_external_inference_readiness_cancellable(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool, AppError> {
    let models_url = config.models_url()?;
    let request = apply_external_auth(client.get(models_url), config.api_key.as_deref());
    let response = tokio::select! {
        _ = cancellation.changed() => return Ok(false),
        result = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send()) => result,
    }
    .map_err(|_| AppError::Inference("External inference readiness check timed out".into()))?
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

    let Some(models) = read_external_models_response_cancellable(response, cancellation).await?
    else {
        return Ok(false);
    };
    if !external_model_is_available(&models, &config.model) {
        return Err(AppError::Inference(format!(
            "External inference model '{}' is not available from the configured endpoint",
            config.model
        )));
    }

    Ok(true)
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

    if *cancellation.borrow() {
        return Ok(());
    }

    let request = apply_external_auth(client.post(url).json(&body), api_key.as_deref());

    let response = tokio::select! {
        _ = cancellation.changed() => return Ok(()),
        result = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send()) => result,
    }
    .map_err(|_| AppError::Inference("External inference request timed out before the response started.".into()))?
    .map_err(|e| AppError::Inference(format!("External inference request failed: {e}")))?
    .error_for_status()
    .map_err(|e| AppError::Inference(format!("External inference request failed: {e}")))?;

    let mut stream = response.bytes_stream();
    let mut total_output_bytes = 0;
    let mut buffer = Vec::new();

    loop {
        let chunk = tokio::select! {
            _ = cancellation.changed() => return Ok(()),
            result = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next()) => match result {
                Ok(chunk) => chunk,
                Err(_) => {
                    send_inference_event(
                        cancellation,
                        events,
                        InferenceEvent::StreamError(
                            "External inference stream timed out while waiting for data.".into(),
                        ),
                    )
                    .await;
                    return Ok(());
                }
            },
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
            if sse_complete_event_exceeds_limit(event_end + delimiter_len) {
                send_inference_event(
                    cancellation,
                    events,
                    InferenceEvent::StreamError(
                        "External inference sent an oversized complete SSE event.".into(),
                    ),
                )
                .await;
                return Ok(());
            }

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
            let data = match sse_event_data(event) {
                Ok(data) => data,
                Err(error) => {
                    send_inference_event(
                        cancellation,
                        events,
                        InferenceEvent::StreamError(error.to_string()),
                    )
                    .await;
                    return Ok(());
                }
            };
            buffer.drain(..event_end + delimiter_len);

            match data.as_deref().map(str::trim) {
                Some("[DONE]") => {
                    send_inference_event(cancellation, events, InferenceEvent::Done).await;
                    return Ok(());
                }
                Some(data) => match external_sse_token(data) {
                    Ok(Some(token)) => {
                        if sse_token_exceeds_limit(&token) {
                            send_inference_event(
                                cancellation,
                                events,
                                InferenceEvent::StreamError(
                                    "External inference sent an oversized SSE token.".into(),
                                ),
                            )
                            .await;
                            return Ok(());
                        }
                        if sse_output_exceeds_limit(total_output_bytes, &token) {
                            send_inference_event(
                                cancellation,
                                events,
                                InferenceEvent::StreamError(
                                    "External inference exceeded the total output limit.".into(),
                                ),
                            )
                            .await;
                            return Ok(());
                        }
                        total_output_bytes += token.len();
                        if !send_inference_event(cancellation, events, InferenceEvent::Token(token))
                            .await
                        {
                            return Ok(());
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        send_inference_event(
                            cancellation,
                            events,
                            InferenceEvent::StreamError(error.to_string()),
                        )
                        .await;
                        return Ok(());
                    }
                },
                None => {}
            }
        }

        if sse_incomplete_buffer_exceeds_limit(&buffer) {
            send_inference_event(
                cancellation,
                events,
                InferenceEvent::StreamError(
                    "External inference sent an oversized incomplete SSE event.".into(),
                ),
            )
            .await;
            return Ok(());
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
    let stream = match stream_result {
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

    forward_local_responses(stream, cancellation, events).await
}

async fn forward_local_responses(
    mut stream: impl futures_util::Stream<Item = mistralrs::Response> + Unpin,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    use futures_util::StreamExt;

    loop {
        let response = tokio::select! {
            _ = cancellation.changed() => return Ok(()),
            response = stream.next() => response,
        };
        match response {
            Some(mistralrs::Response::Chunk(chunk)) => {
                if let Some(choice) = chunk.choices.into_iter().next() {
                    if let Some(token) = choice.delta.content.filter(|token| !token.is_empty())
                        && !send_inference_event(cancellation, events, InferenceEvent::Token(token))
                            .await
                    {
                        return Ok(());
                    }
                    if choice.finish_reason.is_some() {
                        send_inference_event(cancellation, events, InferenceEvent::Done).await;
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

        "generation_model" if is_valid_hugging_face_model_id(value) => Ok(()),
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

fn register_active_query(
    queries: &Arc<std::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    request_id: &str,
) -> Result<tokio::sync::watch::Receiver<bool>, AppError> {
    if request_id.is_empty() {
        return Err(AppError::InvalidInput(
            "Query request ID must not be empty.".into(),
        ));
    }

    let (sender, receiver) = tokio::sync::watch::channel(false);
    let mut queries = queries
        .lock()
        .map_err(|_| AppError::Internal("Query cancellation state is poisoned".into()))?;
    if queries.contains_key(request_id) {
        return Err(AppError::InvalidInput(
            "A query with this request ID is already active.".into(),
        ));
    }
    queries.insert(request_id.to_owned(), sender);
    Ok(receiver)
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

async fn invalidate_llm_readiness(
    app: &tauri::AppHandle,
    state: &AppState,
    clear_local_model: bool,
) -> bool {
    let mut initialization = state.llm_initialization.lock().await;
    invalidate_llm_initialization(&mut initialization);
    let should_start = !initialization.in_progress;
    if should_start {
        initialization.in_progress_generation = None;
    } else {
        initialization.restart_requested = true;
    }
    if clear_local_model {
        *state.local_llm.write().await = None;
    }
    let _ = app.emit("llm_unready", ());
    should_start
}

async fn request_llm_initialization_from_persisted_state(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), AppError> {
    let models_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?
        .join("models");
    request_llm_initialization(app, state, models_dir).await;
    Ok(())
}

async fn invalidate_external_readiness_before_change(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    let mode = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string());
    if mode == "external" {
        return Ok(invalidate_llm_readiness(app, state, false).await);
    }

    Ok(false)
}

async fn set_local_model_setting_and_restart_if_active(
    app: &tauri::AppHandle,
    state: &AppState,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    let settings = AppSettingsRepository::new(state.db_pool.clone());
    if settings.get(key).await?.as_deref() == Some(value) {
        return Ok(());
    }

    let local_mode_active = settings
        .get("inference_mode")
        .await?
        .unwrap_or_else(|| "local".to_string())
        == "local";
    let should_start = local_mode_active && invalidate_llm_readiness(app, state, true).await;
    let write_result = settings.set(key, value).await;
    if should_start {
        request_llm_initialization_from_persisted_state(app, state).await?;
    }
    write_result
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
        let mode = match app_settings.get("inference_mode").await {
            Ok(mode) => mode.unwrap_or_else(|| "local".to_string()),
            Err(error) => {
                let message = format!("Failed to read inference mode: {error}");
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
            let model_id = match app_settings.get("generation_model").await? {
                Some(value) if !value.is_empty() => {
                    validate_setting("generation_model", &value)?;
                    value
                }
                _ => "unsloth/Qwen3-1.7B-GGUF".to_string(),
            };
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
                let current_mode = match app_settings.get("inference_mode").await {
                    Ok(mode) => mode.unwrap_or_else(|| "local".to_string()),
                    Err(error) => {
                        let message = format!("Failed to read inference mode: {error}");
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

async fn requeue_indexed_documents_after_vector_loss(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<String>, AppError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    let ids = sqlx::query_scalar::<_, String>("SELECT id FROM documents WHERE status = 'indexed'")
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    if !ids.is_empty() {
        sqlx::query(
            "UPDATE documents
             SET status = 'queued',
                 chunk_count = NULL,
                 embedding_model = NULL,
                 embedding_attempts = 0,
                 processing_started_at = NULL,
                 indexed_at = NULL,
                 error_message = NULL,
                 retry_count = 0
             WHERE status = 'indexed'",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    Ok(ids)
}

fn indexed_chunk_count_mismatch(recorded_count: Option<i64>, indexed_count: Option<usize>) -> bool {
    match recorded_count.and_then(|count| usize::try_from(count).ok()) {
        Some(recorded_count) => indexed_count != Some(recorded_count),
        None => true,
    }
}

async fn requeue_indexed_documents_with_chunk_count_mismatches(
    pool: &sqlx::SqlitePool,
    indexed_chunk_counts: &HashMap<String, usize>,
) -> Result<Vec<String>, AppError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    let indexed_documents = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT id, chunk_count FROM documents WHERE status = 'indexed'",
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(|error| AppError::Database(error.to_string()))?;
    let ids = indexed_documents
        .into_iter()
        .filter_map(|(id, chunk_count)| {
            indexed_chunk_count_mismatch(chunk_count, indexed_chunk_counts.get(&id).copied())
                .then_some(id)
        })
        .collect::<Vec<_>>();

    for id in &ids {
        sqlx::query(
            "UPDATE documents
             SET status = 'queued',
                 chunk_count = NULL,
                 embedding_model = NULL,
                 embedding_attempts = 0,
                 processing_started_at = NULL,
                 indexed_at = NULL,
                 error_message = NULL,
                 retry_count = 0
             WHERE id = ? AND status = 'indexed'",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| AppError::Database(error.to_string()))?;

    Ok(ids)
}

async fn recover_startup_documents(
    pool: &sqlx::SqlitePool,
    vector_db: &VectorDb,
    document_operations: &tokio::sync::RwLock<()>,
) -> Result<HashSet<String>, AppError> {
    let repo = DocumentRepository::new(pool.clone());
    let _document_operation = document_operations.write().await;
    let mut recovered_ids = HashSet::new();

    if !vector_db.chunks_table_exists().await? {
        recovered_ids.extend(requeue_indexed_documents_after_vector_loss(pool).await?);
    } else {
        let indexed_chunk_counts = vector_db.indexed_chunk_counts().await?;
        recovered_ids.extend(
            requeue_indexed_documents_with_chunk_count_mismatches(pool, &indexed_chunk_counts)
                .await?,
        );
    }

    let settings = AppSettingsRepository::new(pool.clone());
    let desired_provenance = desired_indexing_provenance(&settings).await?;
    recovered_ids.extend(
        repo.requeue_indexed_with_stale_provenance(&desired_provenance)
            .await?,
    );
    recovered_ids.extend(
        repo.reset_interrupted_embeddings(MAX_AUTOMATIC_RECOVERY_ATTEMPTS)
            .await?,
    );
    recovered_ids.extend(
        repo.reset_failed_to_queued(MAX_AUTOMATIC_RECOVERY_ATTEMPTS)
            .await?,
    );
    recovered_ids.extend(repo.get_queued_ids().await?);
    Ok(recovered_ids)
}

async fn recover_startup_documents_with_retry(
    pool: sqlx::SqlitePool,
    vector_db: Arc<VectorDb>,
    document_operations: Arc<tokio::sync::RwLock<()>>,
    queue: mpsc::Sender<IngestionJob>,
) {
    let mut attempt = 0_u32;
    loop {
        match recover_startup_documents(&pool, &vector_db, &document_operations).await {
            Ok(recovered_ids) => {
                let recovered_count = recovered_ids.len();
                for id in recovered_ids {
                    if let Err(error) = queue.send(IngestionJob::background(id)).await {
                        error!(
                            "startup recovery — document worker stopped before recovery completed: {error}"
                        );
                        return;
                    }
                }
                if recovered_count > 0 {
                    info!("startup recovery — re-queued {recovered_count} documents");
                } else {
                    debug!("startup recovery — no documents to resume");
                }
                return;
            }
            Err(error) => {
                attempt = attempt.saturating_add(1);
                let delay = registry_read_retry_delay(attempt);
                warn!(
                    "startup recovery — failed to read durable document work: {error}; retrying in {delay:?}"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
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
        let recovery_vector_db = state.vector_db.clone();
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

                    // Recovery begins only after the embedder is available. Database reads
                    // retry with bounded backoff so durable work is never deferred to restart.
                    tauri::async_runtime::spawn(recover_startup_documents_with_retry(
                        embedder_pool,
                        recovery_vector_db,
                        document_operations,
                        embedder_job_queue,
                    ));
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

fn write_document_source_atomically(
    storage_path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), AppError> {
    use std::io::Write as _;

    let parent = storage_path
        .parent()
        .ok_or_else(|| AppError::Internal("Managed document path has no parent".into()))?;
    let filename = storage_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Internal("Managed document path has no file name".into()))?;
    let temporary_path = parent.join(format!(".{filename}.{}.tmp", uuid::Uuid::now_v7()));

    let result = (|| -> std::io::Result<()> {
        let mut source = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        source.write_all(bytes)?;
        source.sync_all()?;
        std::fs::rename(&temporary_path, storage_path)?;
        let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    result.map_err(AppError::from)
}

#[tauri::command]
async fn upload_document(
    app: tauri::AppHandle,
    filename: String,
    file_bytes: Vec<u8>,
    channel: tauri::ipc::Channel<IngestionProgress>,
) -> Result<String, AppError> {
    let file_type = validate_upload(&filename, &file_bytes)?;
    let state = app.state::<AppState>();
    if state.embedder.read().await.is_none() {
        return Err(AppError::Internal(
            "Embedding model not yet loaded. Please wait before uploading documents.".into(),
        ));
    }
    let repo = DocumentRepository::new(state.db_pool.clone());

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let docs_dir = tokio::task::spawn_blocking({
        let app_local_data = app_local_data.clone();
        move || managed_documents_directory(&app_local_data, true)
    })
    .await
    .map_err(|error| {
        AppError::Internal(format!("Managed document directory task panicked: {error}"))
    })??;

    let file_size_bytes = i64::try_from(file_bytes.len())
        .map_err(|_| AppError::InvalidInput("Document is too large to store".into()))?;
    let (hash, file_bytes) = tokio::task::spawn_blocking(move || {
        let mut hasher = Sha512::new();
        hasher.update(&file_bytes);
        (hasher.finalize().to_vec(), file_bytes)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Hashing failed: {e}")))?;

    if let Some(existing) = repo.find_by_hash(&hash).await? {
        return Err(AppError::DuplicateDocument(format!(
            "This file was already uploaded as '{}' (status: {:?})",
            existing.filename, existing.status
        )));
    }

    let doc_id = uuid::Uuid::now_v7().to_string();
    let storage_filename = format!("{}.{}", doc_id, storage_extension(&filename, file_type));
    let storage_path = docs_dir.join(&storage_filename);
    let relative_storage_path = format!("documents/{}", storage_filename);

    tokio::task::spawn_blocking({
        let storage_path = storage_path.clone();
        move || write_document_source_atomically(&storage_path, &file_bytes)
    })
    .await
    .map_err(|error| AppError::Internal(format!("Document write task panicked: {error}")))??;

    let created_at = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string();

    let doc = Document {
        id: doc_id.clone(),
        filename,
        file_type: file_type.to_string(),
        file_size_bytes: Some(file_size_bytes),
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
    let embedder = embedder_opt
        .ok_or_else(|| AppError::Internal("Embedding model not yet loaded. Please wait.".into()))?;
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
                storage_path: document_storage_path(&app_local_data, &doc.storage_path)?
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
async fn list_documents(state: tauri::State<'_, AppState>) -> Result<Vec<Document>, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    repo.list_all().await
}

#[tauri::command]
async fn get_document_preview(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    document_id: String,
) -> Result<DocumentPreview, AppError> {
    let repo = DocumentRepository::new(state.db_pool.clone());
    let document = repo
        .get_by_id(&document_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Document is not available for preview".into()))?;
    validate_document_preview_document(&document)?;

    let app_local_data = app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::Io(e.to_string()))?;
    let storage_path = document.storage_path;
    let file_type = document.file_type;
    let filename = document.filename;

    tokio::task::spawn_blocking(move || {
        let source_path = document_storage_path(&app_local_data, &storage_path)?;
        rag_pipeline::extractor::document_preview(&source_path, &file_type, &filename)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Document preview task panicked: {e}")))?
}

fn validate_document_preview_document(document: &Document) -> Result<(), AppError> {
    if document.status != DocumentStatus::Indexed {
        return Err(AppError::NotFound(
            "Document is not available for preview".into(),
        ));
    }
    if !matches!(document.file_type.as_str(), "text" | "docx") {
        return Err(AppError::InvalidInput(
            "A preview is unavailable for this document type".into(),
        ));
    }

    Ok(())
}

fn managed_documents_directory(
    app_local_data: &std::path::Path,
    create: bool,
) -> Result<std::path::PathBuf, AppError> {
    let app_data_metadata = std::fs::symlink_metadata(app_local_data)
        .map_err(|error| AppError::Io(format!("Could not inspect app data directory: {error}")))?;
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }

    let documents = app_local_data.join("documents");
    match std::fs::symlink_metadata(&documents) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(AppError::Io(
                "Managed documents directory must be a real directory, not a symlink".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            match std::fs::create_dir(&documents) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(AppError::Io(format!(
                        "Could not create documents directory: {error}"
                    )));
                }
            }
        }
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not inspect managed documents directory: {error}"
            )));
        }
    }

    let metadata = std::fs::symlink_metadata(&documents).map_err(|error| {
        AppError::Io(format!(
            "Could not inspect managed documents directory: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Io(
            "Managed documents directory must be a real directory, not a symlink".into(),
        ));
    }
    std::fs::canonicalize(documents)
        .map(strip_unc_prefix)
        .map_err(|error| {
            AppError::Io(format!(
                "Could not resolve managed documents directory: {error}"
            ))
        })
}

fn strip_unc_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy();
        if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(stripped);
        }
    }
    path
}

fn managed_storage_filename(storage_path: &str) -> Result<&str, AppError> {
    let normalized = storage_path
        .strip_prefix("documents/")
        .or_else(|| storage_path.strip_prefix("documents\\"));
    let Some(filename) = normalized else {
        return Err(AppError::Internal(
            "Document has an unsafe storage path".into(),
        ));
    };
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || std::path::Path::new(filename)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !is_managed_document_source_filename(filename)
    {
        return Err(AppError::Internal(
            "Document has an unsafe storage path".into(),
        ));
    }
    Ok(filename)
}

fn document_storage_candidate(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let documents = managed_documents_directory(app_local_data, false)?;
    Ok(documents.join(managed_storage_filename(storage_path)?))
}

fn document_storage_candidate_for_deletion(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<Option<std::path::PathBuf>, AppError> {
    let filename = managed_storage_filename(storage_path)?;
    let app_data_metadata = std::fs::symlink_metadata(app_local_data)
        .map_err(|error| AppError::Io(format!("Could not inspect app data directory: {error}")))?;
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }

    let documents = app_local_data.join("documents");
    match std::fs::symlink_metadata(&documents) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(AppError::Io(
                "Managed documents directory must be a real directory, not a symlink".into(),
            ))
        }
        Ok(_) => std::fs::canonicalize(documents)
            .map(|documents| Some(strip_unc_prefix(documents).join(filename)))
            .map_err(|error| {
                AppError::Io(format!(
                    "Could not resolve managed documents directory: {error}"
                ))
            }),
        Err(error) => Err(AppError::Io(format!(
            "Could not inspect managed documents directory: {error}"
        ))),
    }
}

fn document_storage_path(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let candidate = document_storage_candidate(app_local_data, storage_path)?;
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| {
        AppError::Io(format!(
            "Could not inspect managed document source: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::Io(
            "Managed document source must be a regular file, not a symlink".into(),
        ));
    }

    let documents = candidate
        .parent()
        .ok_or_else(|| AppError::Internal("Managed document path has no parent".into()))?;
    let source = std::fs::canonicalize(&candidate)
        .map(strip_unc_prefix)
        .map_err(|error| {
            AppError::Io(format!(
                "Could not resolve managed document source: {error}"
            ))
        })?;
    if !source.starts_with(documents) {
        return Err(AppError::Io(
            "Managed document source resolves outside the documents directory".into(),
        ));
    }
    Ok(source)
}

fn verify_document_source_integrity(
    app_local_data: &std::path::Path,
    document: &Document,
) -> Result<std::path::PathBuf, AppError> {
    use std::io::Read as _;

    let source = document_storage_path(app_local_data, &document.storage_path)?;
    let expected_size = document.file_size_bytes.ok_or_else(|| {
        AppError::Io(
            "Managed document source has no recorded file size for integrity validation".into(),
        )
    })?;
    let expected_hash = document.file_hash.as_deref().ok_or_else(|| {
        AppError::Io("Managed document source has no recorded hash for integrity validation".into())
    })?;

    let mut file = std::fs::File::open(&source).map_err(|error| {
        AppError::Io(format!(
            "Could not open managed document source for integrity validation: {error}"
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::Io(format!(
            "Could not inspect managed document source for integrity validation: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::Io(
            "Managed document source must be a regular file for integrity validation".into(),
        ));
    }
    if i64::try_from(metadata.len()).ok() != Some(expected_size) {
        return Err(AppError::Io(
            "Managed document source size does not match its recorded metadata".into(),
        ));
    }

    let mut hasher = Sha512::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Io(format!(
                "Could not read managed document source for integrity validation: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().as_slice() != expected_hash {
        return Err(AppError::Io(
            "Managed document source hash does not match its recorded metadata".into(),
        ));
    }

    Ok(source)
}

async fn finish_document_deletion(
    repo: &DocumentRepository,
    vector_db: &VectorDb,
    app_local_data: &std::path::Path,
    document: &Document,
) -> Result<(), AppError> {
    vector_db.delete_chunks(&document.id).await?;

    if let Some(source_path) =
        document_storage_candidate_for_deletion(app_local_data, &document.storage_path)?
    {
        match tokio::fs::symlink_metadata(&source_path).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                tokio::fs::remove_file(source_path)
                    .await
                    .map_err(|error| AppError::Io(error.to_string()))?;
            }
            Ok(_) => {
                return Err(AppError::Io(
                    "Managed document source must be a regular file, not a symlink".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::Io(error.to_string())),
        }
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

async fn reconcile_orphan_document_sources(
    repo: &DocumentRepository,
    app_local_data: &std::path::Path,
) -> Result<usize, AppError> {
    let registered_filenames: HashSet<String> = repo
        .list_all()
        .await?
        .into_iter()
        .filter_map(|document| {
            managed_storage_filename(&document.storage_path)
                .ok()
                .map(str::to_owned)
        })
        .collect();
    let untrusted_docs_dir = app_local_data.join("documents");
    match tokio::fs::symlink_metadata(&untrusted_docs_dir).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(AppError::Io(error.to_string())),
    }
    let docs_dir = tokio::task::spawn_blocking({
        let app_local_data = app_local_data.to_path_buf();
        move || managed_documents_directory(&app_local_data, false)
    })
    .await
    .map_err(|error| {
        AppError::Internal(format!("Managed document directory task panicked: {error}"))
    })??;
    let mut entries = tokio::fs::read_dir(&docs_dir)
        .await
        .map_err(|error| AppError::Io(error.to_string()))?;
    let mut removed = 0;

    while let Some(entry) = entries.next_entry().await? {
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let is_temporary = is_managed_document_temporary_filename(filename);
        if (!is_temporary && !is_managed_document_source_filename(filename))
            || (!is_temporary && registered_filenames.contains(filename))
        {
            continue;
        }

        match entry.file_type().await {
            Ok(file_type) if file_type.is_file() => {}
            Ok(_) => continue,
            Err(error) => {
                warn!("startup source reconciliation — could not inspect {filename}: {error}");
                continue;
            }
        }

        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(
                    "startup source reconciliation — could not remove orphan {filename}: {error}"
                );
            }
        }
    }

    Ok(removed)
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
        document_storage_candidate_for_deletion(&app_local_data, &document.storage_path)?;
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
    validate_query_request_id(&request_id)?;
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
async fn test_external_provider(
    state: tauri::State<'_, AppState>,
    base_url: String,
    api_key: Option<String>,
) -> Result<ExternalProviderConnection, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let candidate_api_key = api_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    let resolved_key = match candidate_api_key {
        Some(key) => Some(key),
        None => read_external_api_key(&repo).await?,
    };
    let config = ExternalInferenceConfig {
        base_url: normalize_external_base_url(&base_url)?,
        model: DEFAULT_EXTERNAL_MODEL.to_string(),
        api_key: resolved_key,
    };
    let models = external_model_ids(&fetch_external_models(&state.http_client, &config).await?);
    if models.is_empty() {
        return Err(AppError::Inference(
            "External provider returned no usable models".into(),
        ));
    }

    Ok(ExternalProviderConnection { models })
}

#[tauri::command]
async fn set_external_provider_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    base_url: String,
    model: String,
) -> Result<(), AppError> {
    let normalized_base_url = normalize_external_base_url(&base_url)?;
    validate_setting("external_model", &model)?;
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let write_result = repo
        .set_external_inference_config(normalized_base_url.as_str(), model.trim())
        .await;
    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }
    write_result
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
        return set_local_model_setting_and_restart_if_active(&app, &state, &key, &value).await;
    }

    let repo = AppSettingsRepository::new(state.db_pool.clone());
    if key == "inference_mode" {
        if inference_mode_is_unchanged(repo.get(&key).await?.as_deref(), &value) {
            return Ok(());
        }
        let should_start = invalidate_llm_readiness(&app, &state, true).await;
        let write_result = repo.set(&key, &value).await;
        if should_start {
            request_llm_initialization_from_persisted_state(&app, &state).await?;
        }
        return write_result;
    }

    if matches!(key.as_str(), "external_base_url" | "external_model") {
        let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
        let write_result = repo.set(&key, &value).await;
        if should_start {
            request_llm_initialization_from_persisted_state(&app, &state).await?;
        }
        return write_result;
    }

    let requeued_ids = if matches!(key.as_str(), "chunk_size" | "chunk_overlap") {
        let _document_operation = state.document_operations.write().await;
        let mut desired_provenance = desired_indexing_provenance(&repo).await?;
        match key.as_str() {
            "chunk_size" => {
                desired_provenance.chunk_size = parse_chunk_setting("chunk_size", &value)?;
            }
            "chunk_overlap" => {
                desired_provenance.chunk_overlap = parse_chunk_setting("chunk_overlap", &value)?;
            }
            _ => unreachable!("chunk settings branch only accepts chunk settings"),
        }
        validate_indexing_provenance(&desired_provenance)?;
        repo.set_indexing_setting_and_requeue_stale_documents(&key, &value, &desired_provenance)
            .await?
    } else {
        repo.set(&key, &value).await?;
        Vec::new()
    };

    enqueue_requeued_documents(state.job_queue.clone(), requeued_ids)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyStorage {
    Keychain,
    Database,
    None,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKeyStatus {
    pub has_key: bool,
    pub storage: ApiKeyStorage,
}

#[tauri::command]
async fn set_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<ApiKeyStatus, AppError> {
    let key_to_save = key.trim().to_string();
    if key_to_save.is_empty() {
        return Err(AppError::InvalidInput("API key cannot be empty".into()));
    }

    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let key_for_keychain = key_to_save.clone();
    let keychain_result = tokio::task::spawn_blocking(move || {
        let entry = keyring::v1::Entry::new("ragtag", "external_api_key")
            .map_err(|e| format!("Entry error: {e}"))?;
        entry.set_password(&key_for_keychain)
            .map_err(|e| format!("Set password error: {e}"))?;
        Ok::<(), String>(())
    })
    .await;

    let storage = match keychain_result {
        Ok(Ok(())) => {
            let _ = repo.delete("external_api_key").await;
            ApiKeyStorage::Keychain
        }
        _ => {
            log::warn!("System keychain is unavailable; falling back to SQLite storage for API key");
            repo.set("external_api_key", &key_to_save).await?;
            ApiKeyStorage::Database
        }
    };

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }

    Ok(ApiKeyStatus {
        has_key: true,
        storage,
    })
}

#[tauri::command]
async fn get_api_key_status(
    state: tauri::State<'_, AppState>,
) -> Result<ApiKeyStatus, AppError> {
    let repo = AppSettingsRepository::new(state.db_pool.clone());
    let has_keychain_key = tokio::task::spawn_blocking(|| {
        let entry = match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => entry,
            Err(_) => return false,
        };
        match entry.get_password() {
            Ok(password) => !password.is_empty(),
            Err(_) => false,
        }
    })
    .await
    .unwrap_or(false);

    if has_keychain_key {
        return Ok(ApiKeyStatus {
            has_key: true,
            storage: ApiKeyStorage::Keychain,
        });
    }

    if let Ok(Some(db_key)) = repo.get("external_api_key").await
        && !db_key.trim().is_empty()
    {
        return Ok(ApiKeyStatus {
            has_key: true,
            storage: ApiKeyStorage::Database,
        });
    }

    Ok(ApiKeyStatus {
        has_key: false,
        storage: ApiKeyStorage::None,
    })
}

#[tauri::command]
async fn delete_api_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, AppError> {
    let should_start = invalidate_external_readiness_before_change(&app, &state).await?;
    let repo = AppSettingsRepository::new(state.db_pool.clone());

    let _ = tokio::task::spawn_blocking(|| {
        if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
            let _ = entry.delete_credential();
        }
    })
    .await;

    let _ = repo.delete("external_api_key").await;

    if should_start {
        request_llm_initialization_from_persisted_state(&app, &state).await?;
    }
    Ok(true)
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

    let (db_size, lancedb_size, docs_size, models_size) =
        tokio::task::spawn_blocking(move || storage_sizes(&app_local_data))
            .await
            .map_err(|error| {
                AppError::Internal(format!("Storage accounting task panicked: {error}"))
            })??;

    Ok(serde_json::json!({
        "database_bytes": db_size,
        "vector_store_bytes": lancedb_size,
        "documents_bytes": docs_size,
        "models_bytes": models_size,
    }))
}

fn storage_sizes(app_local_data: &std::path::Path) -> Result<(u64, u64, u64, u64), AppError> {
    let metadata = std::fs::symlink_metadata(app_local_data)
        .map_err(|error| AppError::Io(format!("Could not inspect app data directory: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }

    Ok((
        managed_file_size(&app_local_data.join("app.db"))?,
        dir_size(&app_local_data.join(".lancedb"))?,
        dir_size(&app_local_data.join("documents"))?,
        dir_size(&app_local_data.join("models"))?,
    ))
}

fn managed_file_size(path: &std::path::Path) -> Result<u64, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(AppError::Io(format!(
                "Managed storage path is not a regular file: {}",
                path.display()
            )))
        }
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(AppError::Io(format!(
            "Could not inspect managed storage file {}: {error}",
            path.display()
        ))),
    }
}

fn dir_size(path: &std::path::Path) -> Result<u64, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not inspect managed storage directory {}: {error}",
                path.display()
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::Io(format!(
            "Managed storage directory is unsafe: {}",
            path.display()
        )));
    }

    let mut total = 0u64;
    let entries = std::fs::read_dir(path).map_err(|error| {
        AppError::Io(format!(
            "Could not read managed storage directory {}: {error}",
            path.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            AppError::Io(format!(
                "Could not inspect an entry in {}: {error}",
                path.display()
            ))
        })?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            AppError::Io(format!(
                "Could not inspect managed storage entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let size = if metadata.is_dir() {
            dir_size(&entry.path())?
        } else if metadata.is_file() {
            metadata.len()
        } else {
            return Err(AppError::Io(format!(
                "Managed storage entry is not a regular file or directory: {}",
                entry.path().display()
            )));
        };
        total = total.checked_add(size).ok_or_else(|| {
            AppError::Io(format!(
                "Managed storage size overflowed for {}",
                path.display()
            ))
        })?;
    }
    Ok(total)
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
    match std::fs::symlink_metadata(models_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::other(
                "Managed models directory must be a real directory, not a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let entries = std::fs::read_dir(models_dir)?;

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
    let app_data_metadata = match std::fs::symlink_metadata(app_local_data) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Io(format!(
                "Could not inspect app data directory: {error}"
            )));
        }
    };
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::Io(
            "App data directory must be a real directory, not a symlink".into(),
        ));
    }
    if std::fs::symlink_metadata(sentinel)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(AppError::Io(
            "Pending reset marker must not be a symlink".into(),
        ));
    }
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
        app_local_data.join("app.db-journal"),
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

    if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
        let _ = entry.delete_credential();
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
    tokio::task::spawn_blocking({
        let app_local_data = app_local_data.clone();
        let sentinel = sentinel.clone();
        move || {
            let metadata = std::fs::symlink_metadata(&app_local_data).map_err(|error| {
                AppError::Io(format!("Could not inspect app data directory: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::Io(
                    "App data directory must be a real directory, not a symlink".into(),
                ));
            }
            if std::fs::symlink_metadata(&sentinel)
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(AppError::Io(
                    "Pending reset marker must not be a symlink".into(),
                ));
            }
            Ok(())
        }
    })
    .await
    .map_err(|error| AppError::Internal(format!("Reset validation task panicked: {error}")))??;
    let payload = serde_json::json!({"delete_models": delete_models});
    tokio::fs::write(&sentinel, payload.to_string()).await?;

    if let Ok(entry) = keyring::v1::Entry::new("ragtag", "external_api_key") {
        let _ = entry.delete_credential();
    }

    info!("reset requested — restarting application");
    app.request_restart();

    Ok(serde_json::json!({"restarting": true}))
}

#[tauri::command]
fn get_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
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
            let chunks_table_exists =
                tauri::async_runtime::block_on(vector_db.chunks_table_exists()).map_err(|error| {
                    Box::new(std::io::Error::other(error.to_string()))
                })?;
            if !chunks_table_exists {
                let requeued = tauri::async_runtime::block_on(
                    requeue_indexed_documents_after_vector_loss(&pool),
                )
                .map_err(|error| Box::new(std::io::Error::other(error.to_string())))?;
                if !requeued.is_empty() {
                    info!(
                        "startup vector recovery — re-queued {} indexed documents after chunks table loss",
                        requeued.len()
                    );
                }
            }
            else {
                let indexed_chunk_counts =
                    tauri::async_runtime::block_on(vector_db.indexed_chunk_counts()).map_err(
                        |error| Box::new(std::io::Error::other(error.to_string())),
                    )?;
                let requeued = tauri::async_runtime::block_on(
                    requeue_indexed_documents_with_chunk_count_mismatches(
                        &pool,
                        &indexed_chunk_counts,
                    ),
                )
                .map_err(|error| Box::new(std::io::Error::other(error.to_string())))?;
                if !requeued.is_empty() {
                    info!(
                        "startup vector recovery — re-queued {} indexed documents with mismatched chunk counts",
                        requeued.len()
                    );
                }
            }
            let deletion_repo = DocumentRepository::new(pool.clone());
            tauri::async_runtime::block_on(reconcile_pending_deletions(
                &deletion_repo,
                &vector_db,
                &app_local_data,
            ));

            match tauri::async_runtime::block_on(reconcile_orphan_document_sources(
                &deletion_repo,
                &app_local_data,
            )) {
                Ok(0) => {}
                Ok(removed) => {
                    info!("startup source reconciliation — removed {removed} orphaned source files");
                }
                Err(error) => {
                    error!("startup source reconciliation — failed: {error}");
                }
            }

            let local_llm = Arc::new(tokio::sync::RwLock::new(None));
            let llm_initialization = Arc::new(tokio::sync::Mutex::new(
                state::LlmInitializationState::default(),
            ));
            let document_operations = Arc::new(tokio::sync::RwLock::new(()));
            let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));

            app.manage(AppState {
                db_pool: pool.clone(),
                job_queue: tx.clone(),
                embedder: embedder.clone(),
                vector_db: vector_db.clone(),
                local_llm: local_llm.clone(),
                llm_initialization,
                document_operations: document_operations.clone(),
                active_queries: active_queries.clone(),
                http_client: reqwest::Client::builder()
                    .use_rustls_tls()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| Box::new(std::io::Error::other(e.to_string())))?,
            });

            let docs_dir = app_local_data.clone();
            let app_handle = app.handle().clone();

            // Background worker: processes documents from the channel
            let embedder_worker = embedder.clone();
            let document_operations_worker = document_operations.clone();
            let job_queue_worker = tx;
            tauri::async_runtime::spawn(async move {
                let repo = DocumentRepository::new(pool.clone());
                let mut registry_read_retries: HashMap<String, u32> = HashMap::new();
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
                            registry_read_retries.remove(&doc_id);
                            let settings_repo = AppSettingsRepository::new(pool.clone());
                            match desired_indexing_provenance(&settings_repo).await {
                                Ok(provenance) => {
                                    let integrity = tokio::task::spawn_blocking({
                                        let docs_dir = docs_dir.clone();
                                        let document = doc.clone();
                                        move || verify_document_source_integrity(&docs_dir, &document)
                                    })
                                    .await
                                    .map_err(|error| {
                                        AppError::Internal(format!(
                                            "Document integrity task panicked: {error}"
                                        ))
                                    });
                                    match integrity {
                                        Ok(Ok(source_path)) => {
                                            rag_pipeline::process_document(
                                                rag_pipeline::DocumentIngestion {
                                                    source_path: &source_path,
                                                    file_type: &doc.file_type,
                                                    document_id: &doc_id,
                                                    provenance,
                                                },
                                                embedder,
                                                &repo,
                                                &vector_db,
                                                Arc::clone(&progress_callback),
                                            )
                                            .await
                                        }
                                        Ok(Err(error)) | Err(error) => Err(error),
                                    }
                                }
                                Err(error) => Err(error),
                            }
                        }
                        Ok(Some(document)) => {
                            registry_read_retries.remove(&doc_id);
                            debug!(
                                "document {doc_id} is {:?}; ignoring stale queue entry",
                                document.status,
                            );
                            continue;
                        }
                        Ok(None) => {
                            registry_read_retries.remove(&doc_id);
                            warn!("document {doc_id} not found in DB — may have been deleted");
                            continue;
                        }
                        Err(error) => {
                            let attempt = {
                                let attempt = registry_read_retries.entry(doc_id.clone()).or_insert(0);
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
            get_document_preview,
            delete_document,
            get_settings,
            set_setting,
            set_api_key,
            test_external_provider,
            set_external_provider_config,
            get_api_key_status,
            delete_api_key,
            get_storage_info,
            compact_vector_store,
            vacuum_database,
            reset_all_data,
            get_platform
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_EXTERNAL_MODEL, ExternalInferenceConfig, ExternalModelsResponse, IngestionJob,
        MAX_AUTOMATIC_RECOVERY_ATTEMPTS, MAX_EXTERNAL_MODEL_ID_BYTES, MAX_EXTERNAL_MODELS,
        MAX_SSE_EVENT_BYTES, MAX_SSE_INCOMPLETE_EVENT_BYTES, MAX_SSE_OUTPUT_BYTES,
        MAX_SSE_TOKEN_BYTES, complete_pending_reset, document_storage_path,
        enqueue_requeued_documents, external_model_ids, external_model_is_available,
        external_sse_token, fetch_external_models, finish_llm_initialization,
        get_platform, indexed_chunk_count_mismatch, inference_mode_is_unchanged, invalidate_llm_initialization,
        is_managed_document_source_filename, llm_initialization_is_current,
        managed_storage_filename, normalize_external_base_url, parse_external_models_response,
        probe_external_inference_readiness_cancellable, reconcile_orphan_document_sources,
        recover_startup_documents, register_active_query, registry_read_retry_delay,
        remove_non_fastembed_models, require_retrieved_citations, sse_complete_event_exceeds_limit,
        sse_event_boundary, sse_event_data, sse_incomplete_buffer_exceeds_limit,
        sse_output_exceeds_limit, sse_token_exceeds_limit, storage_extension,
        read_external_api_key, validate_document_preview_document, validate_query_request, validate_setting,
        validate_upload,
    };
    use domain::{AppError, Document, DocumentStatus};
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tokio::time::timeout;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{Mutex, mpsc},
    };

    fn local_chunk(content: Option<&str>, finish_reason: Option<&str>) -> mistralrs::Response {
        mistralrs::Response::Chunk(mistralrs::ChatCompletionChunkResponse {
            id: "local-stream".into(),
            choices: vec![mistralrs::ChunkChoice {
                finish_reason: finish_reason.map(str::to_owned),
                stop_sequence: None,
                index: 0,
                delta: mistralrs::Delta {
                    content: content.map(str::to_owned),
                    role: "assistant".into(),
                    tool_calls: None,
                    reasoning_content: None,
                },
                logprobs: None,
            }],
            created: 0,
            model: "local-model".into(),
            system_fingerprint: String::new(),
            object: "chat.completion.chunk".into(),
            usage: None,
            adapter_generation: None,
            session_id: None,
        })
    }

    #[tokio::test]
    async fn local_stream_finishes_after_forwarding_final_unicode_content() {
        let stream = futures_util::stream::iter([
            local_chunk(Some("ca"), None),
            local_chunk(Some("fé"), Some("stop")),
            local_chunk(Some("must not appear"), None),
        ]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(4);
        super::forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(
            matches!(receiver.recv().await, Some(super::InferenceEvent::Token(text)) if text == "ca")
        );
        assert!(
            matches!(receiver.recv().await, Some(super::InferenceEvent::Token(text)) if text == "fé")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(super::InferenceEvent::Done)
        ));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn local_stream_accepts_a_tokenless_terminal_chunk() {
        let stream = futures_util::stream::iter([
            local_chunk(Some("42"), None),
            local_chunk(None, Some("length")),
        ]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(3);
        super::forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(
            matches!(receiver.recv().await, Some(super::InferenceEvent::Token(text)) if text == "42")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(super::InferenceEvent::Done)
        ));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn local_stream_rejects_eof_without_a_completion_marker() {
        let stream = futures_util::stream::iter([local_chunk(Some("partial"), None)]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(3);
        super::forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(
            matches!(receiver.recv().await, Some(super::InferenceEvent::Token(text)) if text == "partial")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(super::InferenceEvent::StreamError(_))
        ));
        assert!(receiver.recv().await.is_none());
    }

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
            serde_json::from_str(r#"{"data":[{"id":"gpt-4o-mini"},{"id":"  other-model  "}]}"#)
                .unwrap();

        assert!(external_model_is_available(&models, "gpt-4o-mini"));
        assert!(external_model_is_available(&models, "other-model"));
        assert!(!external_model_is_available(&models, "missing-model"));
    }

    #[test]
    fn lists_unique_nonempty_external_models() {
        let models: ExternalModelsResponse = serde_json::from_str(
            r#"{"data":[{"id":"gpt-4o-mini"},{"id":"  "},{"id":"gpt-4o-mini"},{"id":"llama"}]}"#,
        )
        .unwrap();

        assert_eq!(external_model_ids(&models), ["gpt-4o-mini", "llama"]);
    }

    #[test]
    fn bounds_external_query_request_and_models_metadata() {
        assert!(validate_query_request("question", &uuid::Uuid::now_v7().to_string()).is_ok());
        assert!(validate_query_request("question", "not-a-uuid").is_err());
        assert!(
            validate_query_request(
                &"x".repeat(32 * 1024 + 1),
                &uuid::Uuid::now_v7().to_string()
            )
            .is_err()
        );

        let oversized_id = format!(
            r#"{{"data":[{{"id":"{}"}}]}}"#,
            "x".repeat(MAX_EXTERNAL_MODEL_ID_BYTES + 1)
        );
        assert!(parse_external_models_response(oversized_id.as_bytes()).is_err());
        let too_many_models = format!(
            r#"{{"data":[{}]}}"#,
            vec![r#"{"id":"m"}"#; MAX_EXTERNAL_MODELS + 1].join(",")
        );
        assert!(parse_external_models_response(too_many_models.as_bytes()).is_err());
    }

    #[test]
    fn detects_indexed_chunk_count_mismatches() {
        assert!(!indexed_chunk_count_mismatch(Some(2), Some(2)));
        assert!(indexed_chunk_count_mismatch(Some(2), Some(1)));
        assert!(indexed_chunk_count_mismatch(None, Some(0)));
        assert!(indexed_chunk_count_mismatch(Some(-1), None));
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
    fn rejects_generation_model_ids_without_both_components() {
        assert!(validate_setting("generation_model", "unsloth/Qwen3-1.7B-GGUF").is_ok());
        for malformed in [
            "",
            "unsloth",
            "unsloth/",
            "/Qwen3-1.7B-GGUF",
            "org/model/extra",
        ] {
            assert!(validate_setting("generation_model", malformed).is_err());
        }
    }

    #[tokio::test]
    async fn discovers_models_from_a_valid_external_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let read = stream.read(&mut request).await.unwrap();
            assert!(
                std::str::from_utf8(&request[..read])
                    .unwrap()
                    .starts_with("GET /v1/models HTTP/1.1")
            );
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 61\r\nConnection: close\r\n\r\n{\"data\":[{\"id\":\"model-b\"},{\"id\":\"model-a\"},{\"id\":\"model-a\"}]}",
                )
                .await
                .unwrap();
        });
        let config = ExternalInferenceConfig {
            base_url: normalize_external_base_url(&format!("http://{address}/v1")).unwrap(),
            model: DEFAULT_EXTERNAL_MODEL.into(),
            api_key: None,
        };

        let models = fetch_external_models(&reqwest::Client::new(), &config)
            .await
            .unwrap();
        assert_eq!(external_model_ids(&models), ["model-a", "model-b"]);
        response.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_external_models_readiness_probe() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            started_sender.send(()).unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let config = ExternalInferenceConfig {
            base_url: normalize_external_base_url(&format!("http://{address}/v1")).unwrap(),
            model: DEFAULT_EXTERNAL_MODEL.into(),
            api_key: None,
        };
        let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);
        let probe = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut cancellation = cancel_receiver;
            probe_external_inference_readiness_cancellable(&client, &config, &mut cancellation)
                .await
        });

        started_receiver.await.unwrap();
        cancel_sender.send(true).unwrap();
        let result = timeout(Duration::from_secs(1), probe)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!result);
        server.await.unwrap();
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
    fn classifies_utf8_text_and_docx_uploads() {
        assert_eq!(
            validate_upload("advisor.md", b"# Notes\n\nUTF-8 text").unwrap(),
            "text"
        );
        assert_eq!(
            validate_upload("manual.DOCX", b"PK\x03\x04").unwrap(),
            "docx"
        );
        assert!(validate_upload("binary.dat", &[0, 159, 146, 150]).is_err());
    }

    #[test]
    fn preserves_safe_source_extensions_in_storage() {
        assert_eq!(storage_extension("advisor.md", "text"), "md");
        assert_eq!(storage_extension("notes", "text"), "text");
        assert_eq!(storage_extension("manual.DOCX", "docx"), "docx");
    }

    #[test]
    fn recognizes_only_managed_document_source_filenames() {
        let managed = format!("{}.docx", uuid::Uuid::now_v7());
        assert!(is_managed_document_source_filename(&managed));
        assert!(!is_managed_document_source_filename(
            "550e8400-e29b-41d4-a716-446655440000.txt"
        ));
        assert!(!is_managed_document_source_filename(
            "018f8db8-ef20-7d50-b7ad-f4567d0b5f85.archive.tar"
        ));
        assert!(!is_managed_document_source_filename("notes.txt"));
    }

    #[test]
    fn validates_managed_storage_filename_cross_platform() {
        let valid_id = uuid::Uuid::now_v7();
        let valid_filename = format!("{valid_id}.pdf");
        assert_eq!(
            managed_storage_filename(&format!("documents/{valid_filename}")).unwrap(),
            valid_filename.as_str()
        );
        assert_eq!(
            managed_storage_filename(&format!("documents\\{valid_filename}")).unwrap(),
            valid_filename.as_str()
        );
        assert!(managed_storage_filename("documents/../outside.txt").is_err());
        assert!(managed_storage_filename("documents\\..\\outside.txt").is_err());
        assert!(managed_storage_filename("documents/sub/file.pdf").is_err());
        assert!(managed_storage_filename("documents\\sub\\file.pdf").is_err());
        assert!(managed_storage_filename("other/file.pdf").is_err());
    }

    #[test]
    fn reports_current_platform() {
        let platform = get_platform();
        assert!(["macos", "windows", "linux"].contains(&platform));
        #[cfg(target_os = "macos")]
        assert_eq!(platform, "macos");
        #[cfg(target_os = "windows")]
        assert_eq!(platform, "windows");
        #[cfg(target_os = "linux")]
        assert_eq!(platform, "linux");
    }

    #[test]
    fn recognizes_active_inference_mode_as_unchanged() {
        assert!(inference_mode_is_unchanged(Some("local"), "local"));
        assert!(inference_mode_is_unchanged(Some("external"), "external"));
        assert!(inference_mode_is_unchanged(None, "local"));
        assert!(!inference_mode_is_unchanged(Some("local"), "external"));
    }

    #[test]
    fn rejects_duplicate_active_query_request_ids() {
        let active_queries = Arc::new(std::sync::Mutex::new(HashMap::new()));
        assert!(register_active_query(&active_queries, "request-1").is_ok());
        assert!(matches!(
            register_active_query(&active_queries, "request-1"),
            Err(AppError::InvalidInput(_))
        ));
    }
    fn indexed_text_document(filename: &str) -> Document {
        Document {
            id: "document-1".into(),
            filename: filename.into(),
            file_type: "text".into(),
            file_size_bytes: Some(10),
            file_hash: None,
            storage_path: "documents/document-1.txt".into(),
            status: DocumentStatus::Indexed,
            chunk_count: Some(1),
            embedding_model: None,
            embedding_attempts: 1,
            created_at: None,
            processing_started_at: None,
            indexed_at: None,
            indexing_provenance: None,
            error_message: None,
            retry_count: 0,
        }
    }

    #[tokio::test]
    async fn removes_only_unregistered_managed_document_sources() {
        let app_local_data = std::env::temp_dir().join(format!(
            "ragtag-source-reconciliation-{}",
            uuid::Uuid::now_v7()
        ));
        let docs_dir = app_local_data.join("documents");
        tokio::fs::create_dir_all(&docs_dir).await.unwrap();
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let repo = persistence::DocumentRepository::new(pool.clone());
        let registered_filename = format!("{}.txt", uuid::Uuid::now_v7());
        let orphan_filename = format!("{}.txt", uuid::Uuid::now_v7());
        let managed_directory_name = format!("{}.txt", uuid::Uuid::now_v7());
        let mut registered = indexed_text_document("registered.txt");
        registered.storage_path = format!("documents/{registered_filename}");
        repo.insert(&registered).await.unwrap();
        tokio::fs::write(docs_dir.join(&registered_filename), b"registered")
            .await
            .unwrap();
        tokio::fs::write(docs_dir.join(&orphan_filename), b"orphan")
            .await
            .unwrap();
        tokio::fs::write(docs_dir.join("user-note.txt"), b"not managed")
            .await
            .unwrap();
        let temporary_filename = format!(".{orphan_filename}.{}.tmp", uuid::Uuid::now_v7());
        tokio::fs::write(docs_dir.join(&temporary_filename), b"partial")
            .await
            .unwrap();
        tokio::fs::create_dir(docs_dir.join(&managed_directory_name))
            .await
            .unwrap();

        assert_eq!(
            reconcile_orphan_document_sources(&repo, &app_local_data)
                .await
                .unwrap(),
            2
        );
        assert!(docs_dir.join(registered_filename).exists());
        assert!(!docs_dir.join(orphan_filename).exists());
        assert!(docs_dir.join("user-note.txt").exists());
        assert!(!docs_dir.join(temporary_filename).exists());
        assert!(docs_dir.join(managed_directory_name).is_dir());

        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[tokio::test]
    async fn requeues_indexed_documents_when_the_chunks_table_is_missing() {
        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-lost-vectors-{}", uuid::Uuid::now_v7()));
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let repo = persistence::DocumentRepository::new(pool.clone());
        let mut document = indexed_text_document("lost-vectors.txt");
        document.embedding_attempts = 3;
        repo.insert(&document).await.unwrap();
        let vector_db = persistence::VectorDb::init(&app_local_data.join(".lancedb"))
            .await
            .unwrap();
        let document_operations = tokio::sync::RwLock::new(());

        let recovered = recover_startup_documents(&pool, &vector_db, &document_operations)
            .await
            .unwrap();

        assert!(recovered.contains(&document.id));
        assert_eq!(
            repo.get_by_id(&document.id).await.unwrap().unwrap().status,
            DocumentStatus::Queued
        );
        assert_eq!(
            repo.get_by_id(&document.id)
                .await
                .unwrap()
                .unwrap()
                .embedding_attempts,
            0
        );
        drop(vector_db);
        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[tokio::test]
    async fn startup_recovery_caps_interrupted_embedding_attempts() {
        let app_local_data = std::env::temp_dir().join(format!(
            "ragtag-interrupted-embeddings-{}",
            uuid::Uuid::now_v7()
        ));
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let repo = persistence::DocumentRepository::new(pool.clone());

        let mut retryable = indexed_text_document("retryable.txt");
        retryable.id = "retryable".into();
        retryable.status = DocumentStatus::Embedding;
        retryable.embedding_attempts = MAX_AUTOMATIC_RECOVERY_ATTEMPTS - 1;
        repo.insert(&retryable).await.unwrap();

        let mut exhausted = indexed_text_document("exhausted.txt");
        exhausted.id = "exhausted".into();
        exhausted.status = DocumentStatus::Embedding;
        exhausted.embedding_attempts = MAX_AUTOMATIC_RECOVERY_ATTEMPTS;
        repo.insert(&exhausted).await.unwrap();

        let vector_db = persistence::VectorDb::init(&app_local_data.join(".lancedb"))
            .await
            .unwrap();
        let document_operations = tokio::sync::RwLock::new(());

        let recovered = recover_startup_documents(&pool, &vector_db, &document_operations)
            .await
            .unwrap();

        assert!(recovered.contains(&retryable.id));
        assert!(!recovered.contains(&exhausted.id));
        assert_eq!(
            repo.get_by_id(&retryable.id).await.unwrap().unwrap().status,
            DocumentStatus::Queued
        );
        assert_eq!(
            repo.get_by_id(&exhausted.id).await.unwrap().unwrap().status,
            DocumentStatus::Failed
        );

        drop(vector_db);
        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[test]
    fn authorizes_only_indexed_text_and_docx_documents_for_preview() {
        let mut document = indexed_text_document("notes.txt");
        assert!(validate_document_preview_document(&document).is_ok());

        document.filename = "notes.md".into();
        assert!(validate_document_preview_document(&document).is_ok());

        document.file_type = "docx".into();
        document.filename = "manual.docx".into();
        assert!(validate_document_preview_document(&document).is_ok());

        document.file_type = "pdf".into();
        assert!(matches!(
            validate_document_preview_document(&document),
            Err(AppError::InvalidInput(_))
        ));

        document.status = DocumentStatus::Embedding;
        assert!(matches!(
            validate_document_preview_document(&document),
            Err(AppError::NotFound(_))
        ));
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
    fn reset_removes_the_sqlite_rollback_journal() {
        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-reset-journal-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&app_local_data).unwrap();
        let journal = app_local_data.join("app.db-journal");
        let sentinel = app_local_data.join(".reset_pending");
        std::fs::write(&journal, b"journal").unwrap();
        std::fs::write(&sentinel, r#"{"delete_models":false}"#).unwrap();

        complete_pending_reset(&app_local_data, &sentinel).unwrap();

        assert!(!journal.exists());
        assert!(!sentinel.exists());
        std::fs::remove_dir_all(app_local_data).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_managed_document_sources_and_models_directory() {
        use std::os::unix::fs::symlink;

        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-symlink-safety-{}", uuid::Uuid::now_v7()));
        let documents = app_local_data.join("documents");
        let outside = app_local_data.join("outside.txt");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        let filename = format!("{}.txt", uuid::Uuid::now_v7());
        symlink(&outside, documents.join(&filename)).unwrap();
        assert!(document_storage_path(&app_local_data, &format!("documents/{filename}")).is_err());
        assert!(managed_storage_filename("documents/../outside.txt").is_err());
        let linked_app_data =
            std::env::temp_dir().join(format!("ragtag-app-data-link-{}", uuid::Uuid::now_v7()));
        symlink(&app_local_data, &linked_app_data).unwrap();
        assert!(document_storage_path(&linked_app_data, &format!("documents/{filename}")).is_err());
        std::fs::remove_file(linked_app_data).unwrap();

        let external_models =
            std::env::temp_dir().join(format!("ragtag-external-models-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&external_models).unwrap();
        symlink(&external_models, app_local_data.join("models")).unwrap();
        assert!(remove_non_fastembed_models(&app_local_data.join("models")).is_err());

        std::fs::remove_dir_all(app_local_data).unwrap();
        std::fs::remove_dir_all(external_models).unwrap();
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
        let data = sse_event_data(event).unwrap();
        assert_eq!(
            data.as_deref(),
            Some("{\"choices\":[{\"delta\":{\"content\":\"café\"}}]}")
        );
        assert_eq!(
            external_sse_token(data.as_deref().unwrap()).unwrap(),
            Some("café".into())
        );
    }

    #[test]
    fn joins_multiline_sse_data_and_accepts_metadata_chunks() {
        let event = "event: message\nvendor-field: metadata\ndata: {\"choices\":[{\"delta\":\ndata: {\"content\":\"joined\"}}]}";
        let data = sse_event_data(event).unwrap().unwrap();
        assert_eq!(external_sse_token(&data).unwrap(), Some("joined".into()));
        assert_eq!(
            external_sse_token(r#"{"choices":[],"usage":{"total_tokens":1}}"#).unwrap(),
            None
        );
        assert_eq!(
            external_sse_token(r#"{"choices":[{"delta":{"content":null}}]}"#).unwrap(),
            None
        );
        assert!(external_sse_token("{not json}").is_err());
        assert!(external_sse_token(r#"{"unexpected":true}"#).is_err());
    }

    #[test]
    fn caps_only_incomplete_sse_events_over_the_limit() {
        let mut buffer = vec![0; MAX_SSE_INCOMPLETE_EVENT_BYTES];
        assert!(!sse_incomplete_buffer_exceeds_limit(&buffer));
        buffer.push(0);
        assert!(sse_incomplete_buffer_exceeds_limit(&buffer));
    }

    #[test]
    fn bounds_complete_sse_events_tokens_and_output() {
        assert!(!sse_complete_event_exceeds_limit(MAX_SSE_EVENT_BYTES));
        assert!(sse_complete_event_exceeds_limit(MAX_SSE_EVENT_BYTES + 1));
        assert!(!sse_token_exceeds_limit(&"x".repeat(MAX_SSE_TOKEN_BYTES)));
        assert!(sse_token_exceeds_limit(
            &"x".repeat(MAX_SSE_TOKEN_BYTES + 1)
        ));
        assert!(!sse_output_exceeds_limit(MAX_SSE_OUTPUT_BYTES - 1, "x"));
        assert!(sse_output_exceeds_limit(MAX_SSE_OUTPUT_BYTES, "x"));
    }

    #[test]
    fn caps_registry_read_retry_delay() {
        assert_eq!(registry_read_retry_delay(1), Duration::from_millis(250));
        assert_eq!(registry_read_retry_delay(3), Duration::from_secs(1));
        assert_eq!(registry_read_retry_delay(u32::MAX), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn read_external_api_key_falls_back_to_sqlite() {
        let app_local_data = std::env::temp_dir().join(format!("ragtag-test-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&app_local_data).await.unwrap();
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let settings = persistence::AppSettingsRepository::new(pool.clone());

        settings.set("external_api_key", "sk-test-fallback-key").await.unwrap();
        let retrieved = read_external_api_key(&settings).await.unwrap();
        assert!(retrieved.is_some());

        settings.delete("external_api_key").await.unwrap();
        let _ = pool.close().await;
        let _ = tokio::fs::remove_dir_all(&app_local_data).await;
    }
}
