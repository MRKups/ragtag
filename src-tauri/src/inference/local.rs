use domain::AppError;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::{InferenceEvent, send_inference_event};

pub async fn stream_local_inference(
    model: Arc<mistralrs::Model>,
    system_prompt: String,
    context: String,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    if *cancellation.borrow() {
        return Ok(());
    }
    let request = mistralrs::RequestBuilder::new()
        .add_message(mistralrs::TextMessageRole::System, system_prompt)
        .add_message(mistralrs::TextMessageRole::User, context);
    let stream_result = tokio::select! {
        biased;
        _ = cancellation.changed() => return Ok(()),
        result = model.stream_chat_request(request) => result,
    };
    if *cancellation.borrow() {
        return Ok(());
    }
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

pub async fn forward_local_responses(
    mut stream: impl futures_util::Stream<Item = mistralrs::Response> + Unpin,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) -> Result<(), AppError> {
    use futures_util::StreamExt;

    if *cancellation.borrow() {
        return Ok(());
    }

    loop {
        let response = tokio::select! {
            biased;
            _ = cancellation.changed() => return Ok(()),
            response = stream.next() => response,
        };
        if *cancellation.borrow() {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn local_chunk(content: Option<&str>, finish_reason: Option<&str>) -> mistralrs::Response {
        mistralrs::Response::Chunk(mistralrs::ChatCompletionChunkResponse {
            id: "chunk-1".into(),
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
        forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "ca"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "fé"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Done)
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
        forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "42"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Done)
        ));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn local_stream_rejects_eof_without_a_completion_marker() {
        let stream = futures_util::stream::iter([local_chunk(Some("partial"), None)]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(3);
        forward_local_responses(stream, &mut cancellation, &events)
            .await
            .unwrap();
        drop(events);

        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "partial"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::StreamError(message)) if message == "Local inference stream ended before completion."
        ));
        assert!(receiver.recv().await.is_none());
    }
}
