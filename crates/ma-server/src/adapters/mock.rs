use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;

use ma_core::NormalizedError;
use ma_core::adapter::{
    NormalizedResponse, ProviderAdapter, ProviderCapabilities, StopReason, Usage,
};
use ma_core::normalized::{ContentDelta, NormalizedContent, NormalizedEvent, NormalizedRequest};

pub struct MockAdapter;

impl MockAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProviderAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new(true, true, false, false, false, 0, 0)
    }

    async fn complete(
        &self,
        request: NormalizedRequest,
    ) -> Result<NormalizedResponse, NormalizedError> {
        Ok(NormalizedResponse::new(
            "msg_ma_compat".to_string(),
            request.model_alias,
            "Models Assemble compat-probe OK.".to_string(),
            StopReason::Stop,
            Usage::new(0, 0, 0),
        ))
    }

    async fn stream(
        &self,
        request: NormalizedRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
        NormalizedError,
    > {
        let events = vec![
            NormalizedEvent::MessageStart {
                id: "msg_ma_compat".to_string(),
                model: request.model_alias.clone(),
                extra: HashMap::new(),
            },
            NormalizedEvent::ContentBlockStart {
                index: 0,
                block: NormalizedContent::Text {
                    text: "".to_string(),
                },
                extra: HashMap::new(),
            },
            NormalizedEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: "Models Assemble compat-probe OK.".to_string(),
                },
                extra: HashMap::new(),
            },
            NormalizedEvent::ContentBlockStop {
                index: 0,
                extra: HashMap::new(),
            },
            NormalizedEvent::MessageDelta {
                stop_reason: Some("end_turn".to_string()),
                extra: HashMap::new(),
            },
            NormalizedEvent::MessageStop {
                extra: HashMap::new(),
            },
        ];

        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    fn make_request() -> NormalizedRequest {
        NormalizedRequest::new("assemble-mock".to_string())
    }

    #[test]
    fn mock_adapter_name_is_mock() {
        let adapter = MockAdapter::new();
        assert_eq!(adapter.name(), "mock");
    }

    #[test]
    fn mock_adapter_capabilities() {
        let adapter = MockAdapter::new();
        let caps = adapter.capabilities();
        assert!(caps.streaming);
        assert!(caps.tools);
        assert!(!caps.parallel_tool_calls);
        assert!(!caps.vision);
        assert!(!caps.thinking);
    }

    #[tokio::test]
    async fn mock_adapter_complete_returns_expected_response() {
        let adapter = MockAdapter::new();
        let request = make_request();
        let response = adapter.complete(request).await.unwrap();

        assert_eq!(response.id, "msg_ma_compat");
        assert_eq!(response.model, "assemble-mock");
        assert_eq!(response.content, "Models Assemble compat-probe OK.");
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(response.usage.total_tokens, 0);
    }

    #[tokio::test]
    async fn mock_adapter_stream_returns_expected_events() {
        let adapter = MockAdapter::new();
        let request = make_request();
        let mut stream = adapter.stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::MessageStart { ref id, ref model, .. } if id == "msg_ma_compat" && model == "assemble-mock")
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::ContentBlockStart { index: 0, block: NormalizedContent::Text { ref text }, .. } if text.is_empty())
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::ContentBlockDelta { index: 0, delta: ContentDelta::TextDelta { ref text }, .. } if text == "Models Assemble compat-probe OK.")
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            event,
            NormalizedEvent::ContentBlockStop { index: 0, .. }
        ));

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::MessageDelta { stop_reason: Some(ref reason), .. } if reason == "end_turn")
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::MessageStop { .. }));

        assert!(stream.next().await.is_none());
    }
}
