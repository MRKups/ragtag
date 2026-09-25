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
    use futures_util::StreamExt;

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
    while let Some(chunk) = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next())
        .await
        .map_err(|_| {
            AppError::inference(
                "Read external models response",
                "External inference /models response timed out",
                "Check server responsiveness and network connection to the external provider.",
            )
        })?
    {
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

    parse_external_models_response(&bytes)
}

pub async fn read_external_models_response_cancellable(
    response: reqwest::Response,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Option<ExternalModelsResponse>, AppError> {
    use futures_util::StreamExt;

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
        let chunk = tokio::select! {
            _ = cancellation.changed() => return Ok(None),
            result = tokio::time::timeout(EXTERNAL_STREAM_IDLE_TIMEOUT, stream.next()) => result,
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

pub fn external_model_is_available(models: &ExternalModelsResponse, configured_model: &str) -> bool {
    let configured_model = configured_model.trim();
    models
        .data
        .iter()
        .any(|available_model| available_model.id.trim() == configured_model)
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
    if !external_model_is_available(&models, &config.model) {
        return Err(AppError::inference(
            "Validate external model",
            format!(
                "External inference model '{}' is not available from the configured endpoint",
                config.model
            ),
            "Select an available model from the external provider's model list.",
        ));
    }

    Ok(())
}

pub async fn probe_external_inference_readiness_cancellable(
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
    if !external_model_is_available(&models, &config.model) {
        return Err(AppError::inference(
            "Validate external model",
            format!(
                "External inference model '{}' is not available from the configured endpoint",
                config.model
            ),
            "Select an available model from the external provider's model list.",
        ));
    }

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
    use futures_util::StreamExt;

    if *cancellation.borrow() {
        return Ok(());
    }

    let request = apply_external_auth(client.post(url).json(&body), api_key.as_deref());

    let response = tokio::select! {
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
        let app_local_data = std::env::temp_dir().join(format!("ragtag-test-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&app_local_data).await.unwrap();
        let pool = persistence::init_pool(&app_local_data).await.unwrap();
        let settings = persistence::AppSettingsRepository::new(pool.clone());

        settings.set("external_api_key", "sk-test-fallback").await.unwrap();

        let api_key = read_external_api_key(&settings).await.unwrap();
        assert_eq!(api_key.as_deref(), Some("sk-test-fallback"));

        pool.close().await;
        std::fs::remove_dir_all(app_local_data).unwrap();
    }
}
