pub mod external;
pub mod local;
pub mod sse;

use domain::{AppError, Citation, TokenEvent};
use std::sync::Arc;
use tokio::sync::mpsc;

pub use external::*;
pub use local::*;

pub const INFERENCE_EVENT_BUFFER_CAPACITY: usize = 32;

pub enum InferenceEvent {
    Token(String),
    Done,
    StreamError(String),
    StartupError(AppError),
}

pub enum InferenceProvider {
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
    pub async fn stream(
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
                    .await;
                Ok(())
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

pub async fn send_inference_event(
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
    events: &mpsc::Sender<InferenceEvent>,
    event: InferenceEvent,
) -> bool {
    if *cancellation.borrow() {
        return false;
    }
    tokio::select! {
        biased;
        _ = cancellation.changed() => false,
        result = events.send(event) => result.is_ok(),
    }
}

pub async fn forward_inference_events(
    provider: InferenceProvider,
    mut cancellation: tokio::sync::watch::Receiver<bool>,
    channel: tauri::ipc::Channel<TokenEvent>,
    citations: Vec<Citation>,
) -> Result<(), AppError> {
    if *cancellation.borrow() {
        return Ok(());
    }
    let (events, mut receiver) = mpsc::channel(INFERENCE_EVENT_BUFFER_CAPACITY);
    let producer_cancellation = cancellation.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = provider.stream(producer_cancellation, events.clone()).await {
            let _ = events.send(InferenceEvent::StartupError(error)).await;
        }
    });

    loop {
        if *cancellation.borrow() {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn sending_an_event_prefers_cancellation_and_watch_closure() {
        for cancelled in [false, true] {
            let (cancel, mut cancellation) = tokio::sync::watch::channel(cancelled);
            drop(cancel);
            let (events, mut receiver) = mpsc::channel(1);
            assert!(!send_inference_event(&mut cancellation, &events, InferenceEvent::Done).await);
            drop(events);
            assert!(receiver.recv().await.is_none());
        }
    }

    #[tokio::test]
    async fn sending_an_event_can_be_cancelled_under_backpressure() {
        let (cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(1);
        assert!(
            events
                .send(InferenceEvent::Token("queued".into()))
                .await
                .is_ok()
        );
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        tokio::time::timeout(Duration::from_secs(1), async {
            let (sent, ()) = tokio::join!(
                async {
                    started_sender.send(()).unwrap();
                    send_inference_event(&mut cancellation, &events, InferenceEvent::Done).await
                },
                async {
                    started_receiver.await.unwrap();
                    cancel.send(true).unwrap();
                }
            );
            assert!(!sent);
        })
        .await
        .unwrap();
        drop(events);
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(_))
        ));
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn sending_an_event_returns_false_for_a_closed_receiver() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, receiver) = mpsc::channel(1);
        drop(receiver);
        assert!(!send_inference_event(&mut cancellation, &events, InferenceEvent::Done).await);
    }

    #[tokio::test]
    async fn sending_a_terminal_event_stops_when_a_backpressured_receiver_closes() {
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        let (events, mut receiver) = mpsc::channel(1);
        assert!(
            events
                .send(InferenceEvent::Token("queued".into()))
                .await
                .is_ok()
        );
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        tokio::time::timeout(Duration::from_secs(1), async {
            let (sent, ()) = tokio::join!(
                async {
                    started_sender.send(()).unwrap();
                    send_inference_event(&mut cancellation, &events, InferenceEvent::Done).await
                },
                async {
                    started_receiver.await.unwrap();
                    receiver.close();
                }
            );
            assert!(!sent);
        })
        .await
        .unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(InferenceEvent::Token(_))
        ));
        assert!(receiver.recv().await.is_none());
    }
}
