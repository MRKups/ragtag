use domain::AppError;

pub const MAX_SSE_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_SSE_TOKEN_BYTES: usize = 16 * 1024;
pub const MAX_SSE_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_SSE_INCOMPLETE_EVENT_BYTES: usize = MAX_SSE_EVENT_BYTES;

pub fn sse_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
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

pub fn sse_incomplete_buffer_exceeds_limit(buffer: &[u8]) -> bool {
    buffer.len() > MAX_SSE_INCOMPLETE_EVENT_BYTES
}

pub fn sse_complete_event_exceeds_limit(event_bytes: usize) -> bool {
    event_bytes > MAX_SSE_EVENT_BYTES
}

pub fn sse_token_exceeds_limit(token: &str) -> bool {
    token.len() > MAX_SSE_TOKEN_BYTES
}

pub fn sse_output_exceeds_limit(total_output_bytes: usize, token: &str) -> bool {
    total_output_bytes.saturating_add(token.len()) > MAX_SSE_OUTPUT_BYTES
}

pub fn sse_event_data(event: &str) -> Result<Option<String>, AppError> {
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

pub fn external_sse_token(data: &str) -> Result<Option<String>, AppError> {
    let payload: serde_json::Value = serde_json::from_str(data).map_err(|_| {
        AppError::inference(
            "Parse SSE stream event",
            "External inference sent malformed SSE JSON payload",
            "Verify that the external inference provider is OpenAI-compatible.",
        )
    })?;
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("External provider returned an error.");
        return Err(AppError::inference(
            "Stream external inference",
            format!("External provider error: {message}"),
            "Verify external provider API keys, quota, and server logs.",
        ));
    }
    let choices = payload
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            AppError::inference(
                "Parse SSE stream event",
                "External inference sent an SSE payload without choices",
                "Verify that the external model supports chat streaming.",
            )
        })?;
    let Some(choice) = choices.first().and_then(serde_json::Value::as_object) else {
        return Ok(None);
    };
    let delta = choice
        .get("delta")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            AppError::inference(
                "Parse SSE stream event",
                "External inference sent an SSE payload with an invalid delta",
                "Verify that the external model supports OpenAI-compatible streaming.",
            )
        })?;

    match delta.get("content") {
        Some(serde_json::Value::String(token)) => Ok((!token.is_empty()).then(|| token.clone())),
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(_) => Err(AppError::inference(
            "Parse SSE stream event",
            "External inference sent an SSE payload with a non-text token",
            "Ensure the external model generates standard text tokens.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sse_only_after_complete_utf8_event() {
        let complete = "data: {\"choices\":[{\"delta\":{\"content\":\"test\"}}]}\n\n";
        assert_eq!(sse_event_boundary(complete.as_bytes()), Some((48, 2)));
        let data = sse_event_data(complete).unwrap().unwrap();
        assert_eq!(external_sse_token(&data).unwrap().as_deref(), Some("test"));

        let crlf = "data: {\"choices\":[{\"delta\":{\"content\":\"crlf\"}}]}\r\n\r\n";
        assert_eq!(sse_event_boundary(crlf.as_bytes()), Some((48, 4)));
        let data = sse_event_data(crlf).unwrap().unwrap();
        assert_eq!(external_sse_token(&data).unwrap().as_deref(), Some("crlf"));
    }

    #[test]
    fn bounds_complete_sse_events_tokens_and_output() {
        assert!(sse_complete_event_exceeds_limit(MAX_SSE_EVENT_BYTES + 1));
        assert!(!sse_complete_event_exceeds_limit(MAX_SSE_EVENT_BYTES));

        let oversized_token = "a".repeat(MAX_SSE_TOKEN_BYTES + 1);
        assert!(sse_token_exceeds_limit(&oversized_token));
        assert!(!sse_token_exceeds_limit("normal"));

        assert!(sse_output_exceeds_limit(MAX_SSE_OUTPUT_BYTES, "a"));
        assert!(!sse_output_exceeds_limit(MAX_SSE_OUTPUT_BYTES - 1, "a"));
    }

    #[test]
    fn caps_only_incomplete_sse_events_over_the_limit() {
        let over = vec![b'a'; MAX_SSE_INCOMPLETE_EVENT_BYTES + 1];
        assert!(sse_incomplete_buffer_exceeds_limit(&over));

        let ok = vec![b'a'; MAX_SSE_INCOMPLETE_EVENT_BYTES];
        assert!(!sse_incomplete_buffer_exceeds_limit(&ok));
    }

    #[test]
    fn joins_multiline_sse_data_and_accepts_metadata_chunks() {
        let multiline =
            ": comment\ndata: {\"choices\":[{\"delta\":\ndata: {\"content\":\"joined\"}}]}\n\n";
        let data = sse_event_data(multiline).unwrap().unwrap();
        assert_eq!(
            external_sse_token(&data).unwrap().as_deref(),
            Some("joined")
        );

        let metadata = "data: {\"choices\":[{\"delta\":{}}]}\n\n";
        let data = sse_event_data(metadata).unwrap().unwrap();
        assert_eq!(external_sse_token(&data).unwrap(), None);
    }
}
