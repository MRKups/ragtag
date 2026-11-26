use std::sync::Arc;
use tokio::sync::mpsc;

use super::{InferenceEvent, send_inference_event};

pub fn local_generation_request(
    system_prompt: String,
    context: String,
) -> mistralrs::RequestBuilder {
    mistralrs::RequestBuilder::new()
        .add_message(mistralrs::TextMessageRole::System, system_prompt)
        .add_message(mistralrs::TextMessageRole::User, context)
        .enable_thinking(false)
}

pub async fn stream_local_inference(
    model: Arc<mistralrs::Model>,
    system_prompt: String,
    context: String,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) {
    if *cancellation.borrow() {
        return;
    }
    let request = local_generation_request(system_prompt, context);
    let stream_result = tokio::select! {
        biased;
        _ = cancellation.changed() => return,
        result = model.stream_chat_request(request) => result,
    };
    if *cancellation.borrow() {
        return;
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
            return;
        }
    };

    forward_local_responses(stream, cancellation, events).await
}

pub async fn forward_local_responses(
    mut stream: impl futures_util::Stream<Item = mistralrs::Response> + Unpin,
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
) {
    use futures_util::StreamExt;

    if *cancellation.borrow() {
        return;
    }

    let terminal = loop {
        let response = tokio::select! {
            biased;
            _ = cancellation.changed() => return,
            response = stream.next() => response,
        };
        if *cancellation.borrow() {
            return;
        }
        match response {
            Some(mistralrs::Response::Chunk(chunk)) => {
                if let Some(choice) = chunk.choices.into_iter().next() {
                    if let Some(token) = choice.delta.content.filter(|token| !token.is_empty())
                        && !send_inference_event(cancellation, events, InferenceEvent::Token(token))
                            .await
                    {
                        return;
                    }
                    if choice.finish_reason.is_some() {
                        break InferenceEvent::Done;
                    }
                }
            }
            Some(mistralrs::Response::Done(_)) => {
                break InferenceEvent::Done;
            }
            Some(mistralrs::Response::ModelError(message, _)) => {
                break InferenceEvent::StreamError(message);
            }
            Some(mistralrs::Response::InternalError(error))
            | Some(mistralrs::Response::ValidationError(error)) => {
                break InferenceEvent::StreamError(error.to_string());
            }
            None => {
                break InferenceEvent::StreamError(
                    "Local inference stream ended before completion.".into(),
                );
            }
            Some(_) => {}
        }
    };
    send_inference_event(cancellation, events, terminal).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_request_disables_hidden_thinking_without_changing_source_context() {
        use mistralrs::RequestLike;

        let mut request = local_generation_request(
            "Answer using the source citations.".into(),
            "[Source 1, Page 7]\nRetained evidence.\nQuestion?".into(),
        );
        assert_eq!(request.messages_ref().len(), 2);
        assert_eq!(
            request.messages_ref()[1]["content"]
                .as_ref()
                .left()
                .map(|content| content.as_str()),
            Some("[Source 1, Page 7]\nRetained evidence.\nQuestion?")
        );
        let mistralrs::RequestMessage::Chat {
            enable_thinking, ..
        } = request.take_messages()
        else {
            panic!("Local generation must use chat messages");
        };
        assert_eq!(enable_thinking, Some(false));
    }

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
        forward_local_responses(stream, &mut cancellation, &events).await;
        drop(events);

        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "ca"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "fé"
        ));
        assert!(matches!(receiver.recv().await, Some(InferenceEvent::Done)));
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
        forward_local_responses(stream, &mut cancellation, &events).await;
        drop(events);

        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(token)) if token == "42"
        ));
        assert!(matches!(receiver.recv().await, Some(InferenceEvent::Done)));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn local_stream_rejects_eof_without_a_completion_marker() {
        let stream = futures_util::stream::iter([local_chunk(Some("partial"), None)]);
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(3);
        forward_local_responses(stream, &mut cancellation, &events).await;
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

    #[tokio::test]
    async fn local_stream_ignores_empty_content_and_forwards_one_terminal_error() {
        for error in [
            mistralrs::Response::InternalError(Box::new(std::io::Error::other("engine failed"))),
            mistralrs::Response::ValidationError(Box::new(std::io::Error::other("engine failed"))),
        ] {
            let stream = futures_util::stream::iter([
                local_chunk(Some(""), None),
                error,
                local_chunk(Some("must not appear"), Some("stop")),
            ]);
            let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
            let (events, mut receiver) = mpsc::channel(3);
            forward_local_responses(stream, &mut cancellation, &events).await;
            drop(events);

            assert!(matches!(
                receiver.recv().await,
                Some(InferenceEvent::StreamError(message)) if message == "engine failed"
            ));
            assert!(receiver.recv().await.is_none());
        }
    }

    #[tokio::test]
    async fn local_stream_cancellation_and_watch_closure_are_silent() {
        for cancelled in [false, true] {
            let stream =
                futures_util::stream::iter([local_chunk(Some("must not appear"), Some("stop"))]);
            let (cancel, mut cancellation) = tokio::sync::watch::channel(cancelled);
            drop(cancel);
            let (events, mut receiver) = mpsc::channel(2);
            forward_local_responses(stream, &mut cancellation, &events).await;
            drop(events);
            assert!(receiver.recv().await.is_none());
        }
    }

    #[tokio::test]
    async fn local_stream_cancellation_interrupts_a_pending_response() {
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let mut started_sender = Some(started_sender);
        let stream = futures_util::stream::poll_fn(move |_| {
            if let Some(sender) = started_sender.take() {
                sender.send(()).unwrap();
            }
            std::task::Poll::<Option<mistralrs::Response>>::Pending
        });
        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(2);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(
                forward_local_responses(stream, &mut cancellation, &events),
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
    async fn local_stream_stops_polling_when_the_event_receiver_closes() {
        let polls = std::cell::Cell::new(0);
        let stream = futures_util::stream::poll_fn(|_| {
            polls.set(polls.get() + 1);
            std::task::Poll::Ready(Some(local_chunk(Some("unobserved"), None)))
        });
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, receiver) = mpsc::channel(1);
        drop(receiver);

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            forward_local_responses(stream, &mut cancellation, &events),
        )
        .await
        .unwrap();
        assert_eq!(polls.get(), 1);
    }
}
