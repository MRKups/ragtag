use domain::AppError;
use persistence::AppSettingsRepository;
use std::time::Duration;
use tokio::sync::mpsc;

use super::sse::{
    external_sse_token, sse_complete_event_exceeds_limit, sse_event_boundary, sse_event_data,
    sse_incomplete_buffer_exceeds_limit, sse_output_exceeds_limit, sse_token_exceeds_limit,
};
use super::{InferenceEvent, send_inference_event};

pub const DEFAULT_EXTERNAL_BASE_URL: &str = "http://localhost:11434/v1";
pub const DEFAULT_EXTERNAL_MODEL: &str = "gpt-4o-mini";
pub const EXTERNAL_REQUEST_START_TIMEOUT: Duration = Duration::from_secs(15);
pub const EXTERNAL_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_EXTERNAL_MODELS_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_EXTERNAL_MODELS: usize = 512;
pub const MAX_EXTERNAL_MODEL_ID_BYTES: usize = 256;

#[derive(Clone)]
pub struct ExternalInferenceConfig {
    pub base_url: reqwest::Url,
    pub model: String,
    pub api_key: Option<String>,
}

impl ExternalInferenceConfig {
    pub fn endpoint_url(&self, segments: &[&str]) -> Result<reqwest::Url, AppError> {
        let mut url = self.base_url.clone();
        while url.path().ends_with("//") {
            let mut path = url.path_segments_mut().map_err(|_| {
                AppError::invalid_input(
                    "Format external URL",
                    "External base URL cannot be used as an HTTP endpoint",
                    "Verify the base URL is a valid HTTP or HTTPS endpoint without trailing slashes.",
                )
            })?;
            path.pop_if_empty();
        }
        let mut path = url.path_segments_mut().map_err(|_| {
            AppError::invalid_input(
                "Format external URL",
                "External base URL cannot be used as an HTTP endpoint",
                "Verify the base URL is a valid HTTP or HTTPS endpoint.",
            )
        })?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        Ok(url)
    }

    pub fn models_url(&self) -> Result<reqwest::Url, AppError> {
        self.endpoint_url(&["models"])
    }

    pub fn chat_completions_url(&self) -> Result<reqwest::Url, AppError> {
        self.endpoint_url(&["chat", "completions"])
    }
}

#[derive(serde::Deserialize)]
pub struct ExternalModelsResponse {
    pub data: Vec<ExternalModel>,
}

#[derive(serde::Deserialize)]
pub struct ExternalModel {
    pub id: String,
}

pub fn parse_external_models_response(bytes: &[u8]) -> Result<ExternalModelsResponse, AppError> {
    let models = serde_json::from_slice::<ExternalModelsResponse>(bytes).map_err(|error| {
        AppError::inference(
            "Parse external models response",
            format!(
                "External inference readiness check received an invalid /models response: {error}"
            ),
            "Verify the external server returns an OpenAI-compatible /models JSON response.",
        )
    })?;
    if models.data.len() > MAX_EXTERNAL_MODELS {
        return Err(AppError::inference(
            "Validate external models response",
            format!(
                "External inference /models response exceeds the {MAX_EXTERNAL_MODELS} model limit"
            ),
            "Configure a provider endpoint with fewer exposed models or filter models server-side.",
        ));
    }
    if models
        .data
        .iter()
        .any(|model| model.id.len() > MAX_EXTERNAL_MODEL_ID_BYTES)
    {
        return Err(AppError::inference(
            "Validate external models response",
            format!(
                "External inference /models response contains a model ID longer than {MAX_EXTERNAL_MODEL_ID_BYTES} bytes"
            ),
            "Ensure external model IDs conform to standard length limits.",
        ));
    }

    Ok(models)
}

pub fn external_models_response_too_large(response: &reqwest::Response) -> bool {
    response
        .content_length()
        .is_some_and(|length| length > MAX_EXTERNAL_MODELS_RESPONSE_BYTES as u64)
}

pub async fn read_external_models_response(
    response: reqwest::Response,
) -> Result<ExternalModelsResponse, AppError> {
    Ok(read_external_models_response_inner(response, None)
        .await?
        .expect("An uncancellable models response cannot be cancelled"))
}

pub async fn read_external_models_response_cancellable(
    response: reqwest::Response,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Option<ExternalModelsResponse>, AppError> {
    read_external_models_response_inner(response, Some(cancellation)).await
}

async fn read_external_models_response_inner(
    response: reqwest::Response,
    mut cancellation: Option<&mut tokio::sync::watch::Receiver<bool>>,
) -> Result<Option<ExternalModelsResponse>, AppError> {
    use futures_util::StreamExt;

    if cancellation
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return Ok(None);
    }

    if external_models_response_too_large(&response) {
        return Err(AppError::inference(
            "Read external models response",
            format!(
                "External inference /models response exceeds the {} byte limit",
                MAX_EXTERNAL_MODELS_RESPONSE_BYTES
            ),
            "Configure a provider endpoint with a smaller models catalogue.",
        ));
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let next_chunk = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next());
        let chunk = if let Some(cancellation) = cancellation.as_deref_mut() {
            if *cancellation.borrow() {
                return Ok(None);
            }
            tokio::select! {
                biased;
                _ = cancellation.changed() => return Ok(None),
                result = next_chunk => result,
            }
        } else {
            next_chunk.await
        }
        .map_err(|_| {
            AppError::inference(
                "Read external models response",
                "External inference /models response timed out",
                "Check server responsiveness and network connection to the external provider.",
            )
        })?;
        let Some(chunk) = chunk else {
            return parse_external_models_response(&bytes).map(Some);
        };
        let chunk = chunk.map_err(|error| {
            AppError::inference(
                "Read external models response",
                format!(
                    "External inference readiness check received an invalid /models response: {error}"
                ),
                "Verify the external provider endpoint is functioning correctly.",
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_EXTERNAL_MODELS_RESPONSE_BYTES {
            return Err(AppError::inference(
                "Read external models response",
                format!(
                    "External inference /models response exceeds the {} byte limit",
                    MAX_EXTERNAL_MODELS_RESPONSE_BYTES
                ),
                "Configure a provider endpoint with a smaller models catalogue.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
}

pub fn normalize_external_base_url(value: &str) -> Result<reqwest::Url, AppError> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| {
        AppError::invalid_input(
            "Normalize external base URL",
            "External base URL must be a valid HTTP(S) URL",
            "Enter a valid URL starting with http:// or https://.",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::invalid_input(
            "Normalize external base URL",
            "External base URL must be an HTTP(S) URL without credentials, query, or fragment",
            "Provide a clean host and port URL, e.g. http://localhost:11434/v1.",
        ));
    }

    Ok(url)
}

pub fn apply_external_auth(
    request: reqwest::RequestBuilder,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    match api_key.filter(|key| !key.is_empty()) {
        Some(api_key) => request.bearer_auth(api_key),
        None => request,
    }
}

pub async fn read_external_api_key(
    settings: &AppSettingsRepository,
) -> Result<Option<String>, AppError> {
    let from_keychain = tokio::task::spawn_blocking(|| {
        let entry = match keyring::v1::Entry::new("ragtag", "external_api_key") {
            Ok(entry) => entry,
            Err(_) => {
                log::warn!("System keychain entry unavailable");
                return None;
            }
        };
        match entry.get_password() {
            Ok(api_key) if !api_key.is_empty() => Some(api_key),
            Ok(_) | Err(keyring::v1::Error::NoEntry) => None,
            Err(_) => {
                log::warn!("System keychain password read failed");
                None
            }
        }
    })
    .await
    .unwrap_or(None);

    resolve_external_api_key(settings, from_keychain).await
}

// Keep source selection testable without accessing the developer's OS keychain.
async fn resolve_external_api_key(
    settings: &AppSettingsRepository,
    from_keychain: Option<String>,
) -> Result<Option<String>, AppError> {
    if let Some(key) = from_keychain.filter(|key| !key.trim().is_empty()) {
        return Ok(Some(key));
    }

    let from_db = settings
        .get("external_api_key")
        .await?
        .filter(|key| !key.trim().is_empty());

    Ok(from_db)
}

pub async fn external_inference_config(
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

pub fn external_model_is_available(
    models: &ExternalModelsResponse,
    configured_model: &str,
) -> bool {
    let configured_model = configured_model.trim();
    models
        .data
        .iter()
        .any(|available_model| available_model.id.trim() == configured_model)
}

fn require_external_model(
    models: &ExternalModelsResponse,
    configured_model: &str,
) -> Result<(), AppError> {
    if !external_model_is_available(models, configured_model) {
        return Err(AppError::inference(
            "Validate external model",
            format!(
                "External inference model '{configured_model}' is not available from the configured endpoint"
            ),
            "Select an available model from the external provider's model list.",
        ));
    }
    Ok(())
}

pub fn external_model_ids(models: &ExternalModelsResponse) -> Vec<String> {
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

pub async fn fetch_external_models(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
) -> Result<ExternalModelsResponse, AppError> {
    let models_url = config.models_url()?;
    let request = apply_external_auth(client.get(models_url), config.api_key.as_deref());
    let response = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send())
        .await
        .map_err(|_| {
            AppError::inference(
                "Probe external inference",
                "External inference readiness check timed out",
                "Verify the external provider server is running and accessible.",
            )
        })?
        .map_err(|error| {
            AppError::inference(
                "Probe external inference",
                format!("External inference readiness check failed: {error}"),
                "Verify that the external server URL is reachable.",
            )
        })?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        return Err(AppError::inference(
            "Authenticate external inference",
            format!(
                "Authentication required: endpoint returned HTTP {}",
                response.status()
            ),
            "Provide a valid API key in settings.",
        ));
    } else if !response.status().is_success() {
        return Err(AppError::inference(
            "Probe external inference",
            format!(
                "External inference readiness check failed: endpoint returned HTTP {}",
                response.status()
            ),
            "Check provider server logs and ensure the endpoint supports OpenAI-compatible /v1/models.",
        ));
    }

    read_external_models_response(response).await
}

pub async fn probe_external_inference_readiness(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
) -> Result<(), AppError> {
    let models = fetch_external_models(client, config).await?;
    require_external_model(&models, &config.model)
}

pub async fn probe_external_inference_readiness_cancellable(
    client: &reqwest::Client,
    config: &ExternalInferenceConfig,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool, AppError> {
    if *cancellation.borrow() {
        return Ok(false);
    }
    let models_url = config.models_url()?;
    let request = apply_external_auth(client.get(models_url), config.api_key.as_deref());
    let response = tokio::select! {
        biased;
        _ = cancellation.changed() => return Ok(false),
        result = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send()) => result,
    }
    .map_err(|_| {
        AppError::inference(
            "Probe external inference",
            "External inference readiness check timed out",
            "Verify the external provider server is running and accessible.",
        )
    })?
    .map_err(|error| {
        AppError::inference(
            "Probe external inference",
            format!("External inference readiness check failed: {error}"),
            "Verify that the external server URL is reachable.",
        )
    })?;

    if !response.status().is_success() {
        return Err(AppError::inference(
            "Probe external inference",
            format!(
                "External inference readiness check failed: endpoint returned HTTP {}",
                response.status()
            ),
            "Check provider server logs and ensure the endpoint supports OpenAI-compatible /v1/models.",
        ));
    }

    let Some(models) = read_external_models_response_cancellable(response, cancellation).await?
    else {
        return Ok(false);
    };
    require_external_model(&models, &config.model)?;

    Ok(true)
}

pub async fn stream_external_inference(
    client: reqwest::Client,
    url: String,
    body: serde_json::Value,
    api_key: Option<String>,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    if *cancellation.borrow() {
        return Ok(());
    }

    let request = apply_external_auth(client.post(url).json(&body), api_key.as_deref());

    let response = tokio::select! {
        biased;
        _ = cancellation.changed() => return Ok(()),
        result = tokio::time::timeout(EXTERNAL_REQUEST_START_TIMEOUT, request.send()) => result,
    }
    .map_err(|_| {
        AppError::inference(
            "Stream external inference",
            "External inference request timed out before the response started.",
            "Verify that the external inference server is running and responsive.",
        )
    })?
    .map_err(|e| {
        AppError::inference(
            "Stream external inference",
            format!("External inference request failed: {e}"),
            "Check network connectivity or external provider server status.",
        )
    })?
    .error_for_status()
    .map_err(|e| {
        AppError::inference(
            "Stream external inference",
            format!("External inference request returned error status: {e}"),
            "Check provider server logs and verify your model settings.",
        )
    })?;

    forward_external_sse(response.bytes_stream(), cancellation, events).await;
    Ok(())
}

async fn forward_external_sse(
    mut stream: impl futures_util::Stream<Item = Result<impl AsRef<[u8]>, reqwest::Error>> + Unpin,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) {
    use futures_util::StreamExt;

    let mut total_output_bytes = 0;
    let mut buffer = Vec::new();

    let terminal = 'stream: loop {
        if *cancellation.borrow() {
            return;
        }
        let chunk = tokio::select! {
            biased;
            _ = cancellation.changed() => return,
            result = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next()) => match result {
                Ok(chunk) => chunk,
                Err(_) => {
                    break InferenceEvent::StreamError(
                        "External inference stream timed out while waiting for data.".into(),
                    );
                }
            },
        };
        let Some(chunk) = chunk else {
            break InferenceEvent::StreamError(
                "External inference stream ended before its completion marker.".into(),
            );
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                break InferenceEvent::StreamError(format!("Stream error: {error}"));
            }
        };
        buffer.extend_from_slice(chunk.as_ref());

        while let Some((event_end, delimiter_len)) = sse_event_boundary(&buffer) {
            if sse_complete_event_exceeds_limit(event_end + delimiter_len) {
                break 'stream InferenceEvent::StreamError(
                    "External inference sent an oversized complete SSE event.".into(),
                );
            }

            let event = match std::str::from_utf8(&buffer[..event_end]) {
                Ok(event) => event,
                Err(_) => {
                    break 'stream InferenceEvent::StreamError(
                        "External inference sent invalid UTF-8 in a complete event.".into(),
                    );
                }
            };
            let data = sse_event_data(event);
            buffer.drain(..event_end + delimiter_len);

            match data.as_deref().map(str::trim) {
                Some("[DONE]") => {
                    break 'stream InferenceEvent::Done;
                }
                Some(data) => match external_sse_token(data) {
                    Ok(Some(token)) => {
                        if sse_token_exceeds_limit(&token) {
                            break 'stream InferenceEvent::StreamError(
                                "External inference sent an oversized SSE token.".into(),
                            );
                        }
                        if sse_output_exceeds_limit(total_output_bytes, &token) {
                            break 'stream InferenceEvent::StreamError(
                                "External inference exceeded the total output limit.".into(),
                            );
                        }
                        total_output_bytes += token.len();
                        if !send_inference_event(cancellation, events, InferenceEvent::Token(token))
                            .await
                        {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        break 'stream InferenceEvent::StreamError(error.to_string());
                    }
                },
                None => {}
            }
        }

        if sse_incomplete_buffer_exceeds_limit(&buffer) {
            break InferenceEvent::StreamError(
                "External inference sent an oversized incomplete SSE event.".into(),
            );
        }
    };
    send_inference_event(cancellation, events, terminal).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::timeout;

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
        assert!(require_external_model(&models, "  other-model  ").is_ok());
        let error = require_external_model(&models, "missing-model").unwrap_err();
        assert_eq!(error.operation, "Validate external model");
        assert_eq!(
            error.detail,
            "External inference model 'missing-model' is not available from the configured endpoint"
        );
        assert_eq!(
            error.recovery,
            "Select an available model from the external provider's model list."
        );
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
    fn bounds_external_models_metadata() {
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

    fn models_response(body: reqwest::Body) -> reqwest::Response {
        tauri::http::Response::builder().body(body).unwrap().into()
    }

    #[tokio::test]
    async fn both_models_response_readers_keep_success_and_parse_error_contracts() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let json = r#"{"data":[{"id":"configured-model"}]}"#;
        let models = read_external_models_response(models_response(json.into()))
            .await
            .unwrap();
        assert!(require_external_model(&models, "configured-model").is_ok());
        let models = read_external_models_response_cancellable(
            models_response(json.into()),
            &mut cancellation,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(require_external_model(&models, "configured-model").is_ok());

        let error = read_external_models_response(models_response("invalid JSON".into()))
            .await
            .err()
            .unwrap();
        let cancellable_error = read_external_models_response_cancellable(
            models_response("invalid JSON".into()),
            &mut cancellation,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error, cancellable_error);
        assert_eq!(error.operation, "Parse external models response");
    }

    #[tokio::test]
    async fn both_models_response_readers_bound_known_and_streamed_body_sizes() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        for streamed in [false, true] {
            let response = || {
                if streamed {
                    let chunks = futures_util::stream::iter([
                        Ok::<_, std::io::Error>(vec![b'x'; MAX_EXTERNAL_MODELS_RESPONSE_BYTES]),
                        Ok(vec![b'x']),
                    ]);
                    models_response(reqwest::Body::wrap_stream(chunks))
                } else {
                    models_response(vec![b'x'; MAX_EXTERNAL_MODELS_RESPONSE_BYTES + 1].into())
                }
            };
            assert_eq!(external_models_response_too_large(&response()), !streamed);
            let error = read_external_models_response(response())
                .await
                .err()
                .unwrap();
            let cancellable_error =
                read_external_models_response_cancellable(response(), &mut cancellation)
                    .await
                    .err()
                    .unwrap();
            assert_eq!(error, cancellable_error);
            assert_eq!(error.operation, "Read external models response");
            assert_eq!(
                error.detail,
                format!(
                    "External inference /models response exceeds the {MAX_EXTERNAL_MODELS_RESPONSE_BYTES} byte limit"
                )
            );
        }
    }

    #[tokio::test]
    async fn models_response_cancellation_precedes_the_known_size_limit() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(true);
        let response = models_response(vec![b'x'; MAX_EXTERNAL_MODELS_RESPONSE_BYTES + 1].into());
        assert!(
            read_external_models_response_cancellable(response, &mut cancellation)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn models_response_reader_cancels_between_ready_chunks_and_on_watch_closure() {
        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let mut polls = 0;
        let stream = futures_util::stream::poll_fn(move |_| {
            polls += 1;
            assert_eq!(polls, 1, "Cancellation must prevent polling the next chunk");
            cancel.send(true).unwrap();
            std::task::Poll::Ready(Some(Ok::<_, std::io::Error>(b"invalid JSON".to_vec())))
        });
        let response = models_response(reqwest::Body::wrap_stream(stream));
        assert!(
            read_external_models_response_cancellable(response, &mut cancellation)
                .await
                .unwrap()
                .is_none()
        );

        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        drop(cancel);
        let response = models_response("invalid JSON".into());
        assert!(
            read_external_models_response_cancellable(response, &mut cancellation)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn models_response_cancellation_interrupts_a_pending_body_chunk() {
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let mut started_sender = Some(started_sender);
        let stream = futures_util::stream::poll_fn(move |_| {
            if let Some(sender) = started_sender.take() {
                sender.send(()).unwrap();
            }
            std::task::Poll::<Option<Result<Vec<u8>, std::io::Error>>>::Pending
        });
        let response = models_response(reqwest::Body::wrap_stream(stream));
        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (result, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                read_external_models_response_cancellable(response, &mut cancellation),
                async {
                    started_receiver.await.unwrap();
                    cancel.send(true).unwrap();
                }
            )
        })
        .await
        .unwrap();
        assert!(result.unwrap().is_none());
    }

    async fn collect_external_events(chunks: Vec<Vec<u8>>) -> Vec<InferenceEvent> {
        let stream = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, reqwest::Error>));
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(2);
        let (_, collected) = tokio::join!(
            async {
                forward_external_sse(stream, &mut cancellation, &events).await;
                drop(events);
            },
            async {
                let mut collected = Vec::new();
                while let Some(event) = receiver.recv().await {
                    collected.push(event);
                }
                collected
            }
        );
        collected
    }

    #[tokio::test]
    async fn external_stream_keeps_metadata_and_split_unicode_before_done() {
        let input = concat!(
            ": keepalive\n\n",
            "event: message\nid: 1\n\n",
            "data: {\"choices\":[{\"delta\":{}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"café\"}}]}\r\n\r\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"must not appear\"}}]}\n\n"
        );
        let split = input.find('é').unwrap() + 1;
        let events = collect_external_events(vec![
            input.as_bytes()[..split].to_vec(),
            input.as_bytes()[split..].to_vec(),
        ])
        .await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], InferenceEvent::Token(token) if token == "café"));
        assert!(matches!(&events[1], InferenceEvent::Done));
    }

    #[tokio::test]
    async fn external_stream_empty_data_and_provider_errors_are_terminal() {
        for input in [
            "data\n\n",
            "data:\n\n",
            "data: \n\n",
            "data: {\"error\":{\"message\":\"quota exhausted\"}}\n\n",
        ] {
            let events =
                collect_external_events(vec![format!("{input}data: [DONE]\n\n").into_bytes()])
                    .await;
            assert_eq!(events.len(), 1);
            let InferenceEvent::StreamError(message) = &events[0] else {
                panic!("Expected a terminal stream error");
            };
            if input.contains("quota exhausted") {
                assert!(message.contains("External provider error: quota exhausted"));
            } else {
                assert!(message.contains("External inference sent malformed SSE JSON payload"));
            }
        }
    }

    #[tokio::test]
    async fn external_stream_eof_without_done_is_a_terminal_error() {
        let events = collect_external_events(vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n".to_vec(),
        ])
        .await;
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], InferenceEvent::Token(token) if token == "partial"));
        assert!(matches!(
            &events[1],
            InferenceEvent::StreamError(message)
                if message == "External inference stream ended before its completion marker."
        ));
    }

    #[tokio::test]
    async fn external_stream_invalid_utf8_and_body_errors_remain_terminal() {
        let events = collect_external_events(vec![vec![0xff, b'\n', b'\n']]).await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            InferenceEvent::StreamError(message)
                if message == "External inference sent invalid UTF-8 in a complete event."
        ));

        let error = reqwest::Client::new()
            .get("invalid URL")
            .build()
            .unwrap_err();
        let expected = format!("Stream error: {error}");
        let stream = futures_util::stream::iter([Err::<Vec<u8>, _>(error)]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(1);
        forward_external_sse(stream, &mut cancellation, &events).await;
        drop(events);
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::StreamError(message)) if message == expected
        ));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn external_request_setup_errors_still_return_errors_without_stream_events() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(1);
        let error = stream_external_inference(
            reqwest::Client::new(),
            "invalid URL".into(),
            serde_json::json!({"stream": true}),
            None,
            &mut cancellation,
            &events,
        )
        .await
        .unwrap_err();
        drop(events);
        assert_eq!(error.operation, "Stream external inference");
        assert!(
            error
                .detail
                .starts_with("External inference request failed:")
        );
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn external_stream_complete_limit_includes_the_delimiter_not_the_whole_buffer() {
        for delimiter in ["\n\n", "\r\n\r\n"] {
            let event = format!(
                ":{}{delimiter}",
                "x".repeat(super::super::sse::MAX_SSE_EVENT_BYTES - delimiter.len() - 1)
            );
            let events = collect_external_events(vec![
                format!("{event}data: [DONE]{delimiter}").into_bytes(),
            ])
            .await;
            assert_eq!(events.len(), 1);
            assert!(matches!(&events[0], InferenceEvent::Done));

            let events = collect_external_events(vec![format!("x{event}").into_bytes()]).await;
            assert_eq!(events.len(), 1);
            assert!(matches!(
                &events[0],
                InferenceEvent::StreamError(message)
                    if message == "External inference sent an oversized complete SSE event."
            ));
        }
    }

    #[tokio::test]
    async fn external_stream_preserves_token_output_and_incomplete_event_limits() {
        let oversized_token = "x".repeat(super::super::sse::MAX_SSE_TOKEN_BYTES + 1);
        let events = collect_external_events(vec![
            format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{oversized_token}\"}}}}]}}\n\n"
            )
            .into_bytes(),
        ])
        .await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            InferenceEvent::StreamError(message) if message == "External inference sent an oversized SSE token."
        ));

        let token = "x".repeat(super::super::sse::MAX_SSE_TOKEN_BYTES);
        let event =
            format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{token}\"}}}}]}}\n\n");
        let token_count = super::super::sse::MAX_SSE_OUTPUT_BYTES / token.len();
        let events = collect_external_events(vec![event.into_bytes(); token_count + 1]).await;
        assert_eq!(events.len(), token_count + 1);
        assert!(
            events[..token_count]
                .iter()
                .all(|event| matches!(event, InferenceEvent::Token(_)))
        );
        assert!(matches!(
            events.last(),
            Some(InferenceEvent::StreamError(message)) if message == "External inference exceeded the total output limit."
        ));

        let events = collect_external_events(vec![vec![
            b'x';
            super::super::sse::MAX_SSE_INCOMPLETE_EVENT_BYTES
                + 1
        ]])
        .await;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            InferenceEvent::StreamError(message) if message == "External inference sent an oversized incomplete SSE event."
        ));
    }

    #[tokio::test]
    async fn external_stream_cancellation_and_watch_closure_are_silent() {
        for cancelled in [false, true] {
            let stream =
                futures_util::stream::iter([Ok::<_, reqwest::Error>(b"data: [DONE]\n\n".to_vec())]);
            let (cancel, mut cancellation) = tokio::sync::watch::channel(cancelled);
            drop(cancel);
            let (events, mut receiver) = mpsc::channel(1);
            forward_external_sse(stream, &mut cancellation, &events).await;
            drop(events);
            assert!(receiver.recv().await.is_none());
        }
    }

    #[tokio::test]
    async fn external_stream_cancellation_interrupts_a_pending_chunk() {
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let mut started_sender = Some(started_sender);
        let stream = futures_util::stream::poll_fn(move |_| {
            if let Some(sender) = started_sender.take() {
                sender.send(()).unwrap();
            }
            std::task::Poll::<Option<Result<Vec<u8>, reqwest::Error>>>::Pending
        });
        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(1);
        timeout(Duration::from_secs(1), async {
            tokio::join!(
                forward_external_sse(stream, &mut cancellation, &events),
                async {
                    started_receiver.await.unwrap();
                    cancel.send(true).unwrap();
                }
            );
        })
        .await
        .unwrap();
        drop(events);
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn external_stream_stops_polling_when_the_event_receiver_closes() {
        let polls = std::cell::Cell::new(0);
        let stream = futures_util::stream::poll_fn(|_| {
            polls.set(polls.get() + 1);
            std::task::Poll::Ready(Some(Ok::<_, reqwest::Error>(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"unobserved\"}}]}\n\n".to_vec(),
            )))
        });
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, receiver) = mpsc::channel(1);
        drop(receiver);
        timeout(
            Duration::from_secs(1),
            forward_external_sse(stream, &mut cancellation, &events),
        )
        .await
        .unwrap();
        assert_eq!(polls.get(), 1);
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

    #[tokio::test]
    async fn read_external_api_key_falls_back_to_sqlite() {
        let app_local_data =
            std::env::temp_dir().join(format!("ragtag-test-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&app_local_data).await.unwrap();
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let settings = persistence::AppSettingsRepository::new(pool.clone());

        settings
            .set("external_api_key", "sk-test-fallback")
            .await
            .unwrap();

        let api_key = resolve_external_api_key(&settings, None).await.unwrap();
        assert_eq!(api_key.as_deref(), Some("sk-test-fallback"));

        let api_key = resolve_external_api_key(&settings, Some("test-keychain-key".into()))
            .await
            .unwrap();
        assert_eq!(api_key.as_deref(), Some("test-keychain-key"));

        let api_key = resolve_external_api_key(&settings, Some(" ".into()))
            .await
            .unwrap();
        assert_eq!(api_key.as_deref(), Some("sk-test-fallback"));

        settings.delete("external_api_key").await.unwrap();
        assert!(
            resolve_external_api_key(&settings, None)
                .await
                .unwrap()
                .is_none()
        );

        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }
}
