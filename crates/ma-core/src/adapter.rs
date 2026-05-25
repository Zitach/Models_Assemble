use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::NormalizedError;
use crate::normalized::{NormalizedEvent, NormalizedRequest};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tools: bool,
    pub parallel_tool_calls: bool,
    pub vision: bool,
    pub thinking: bool,
    pub max_context_tokens: u32,
    pub max_output_tokens: u32,
}

impl ProviderCapabilities {
    pub fn new(
        streaming: bool,
        tools: bool,
        parallel_tool_calls: bool,
        vision: bool,
        thinking: bool,
        max_context_tokens: u32,
        max_output_tokens: u32,
    ) -> Self {
        Self {
            streaming,
            tools,
            parallel_tool_calls,
            vision,
            thinking,
            max_context_tokens,
            max_output_tokens,
        }
    }
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ProviderHealth {
    pub healthy: bool,
    pub last_check: Option<Instant>,
    pub latency: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NormalizedResponse {
    pub id: String,
    pub model: String,
    pub content: String,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

impl NormalizedResponse {
    pub fn new(
        id: String,
        model: String,
        content: String,
        stop_reason: StopReason,
        usage: Usage,
    ) -> Self {
        Self {
            id,
            model,
            content,
            stop_reason,
            usage,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    #[default]
    Stop,
    Length,
    ToolCall,
    ContentFilter,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl Usage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32, total_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }
    }
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn complete(
        &self,
        request: NormalizedRequest,
    ) -> Result<NormalizedResponse, NormalizedError>;
    async fn stream(
        &self,
        request: NormalizedRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
        NormalizedError,
    >;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    struct MockAdapter {
        name: String,
        caps: ProviderCapabilities,
    }

    impl MockAdapter {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                caps: ProviderCapabilities::default(),
            }
        }
    }

    #[async_trait]
    impl ProviderAdapter for MockAdapter {
        fn name(&self) -> &str {
            &self.name
        }

        fn capabilities(&self) -> ProviderCapabilities {
            self.caps.clone()
        }

        async fn complete(
            &self,
            _request: NormalizedRequest,
        ) -> Result<NormalizedResponse, NormalizedError> {
            Ok(NormalizedResponse {
                id: "test-id".to_string(),
                model: "test-model".to_string(),
                content: "hello".to_string(),
                stop_reason: StopReason::Stop,
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
            })
        }

        async fn stream(
            &self,
            _request: NormalizedRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
            NormalizedError,
        > {
            let event = NormalizedEvent::MessageStop {
                extra: std::collections::HashMap::new(),
            };
            Ok(Box::pin(futures_util::stream::once(async { Ok(event) })))
        }
    }

    #[test]
    fn provider_capabilities_default_is_conservative() {
        let caps = ProviderCapabilities::default();
        assert!(!caps.streaming);
        assert!(!caps.tools);
        assert!(!caps.parallel_tool_calls);
        assert!(!caps.vision);
        assert!(!caps.thinking);
        assert_eq!(caps.max_context_tokens, 0);
        assert_eq!(caps.max_output_tokens, 0);
    }

    #[test]
    fn provider_health_default_is_unhealthy() {
        let health = ProviderHealth::default();
        assert!(!health.healthy);
        assert!(health.last_check.is_none());
        assert!(health.latency.is_none());
    }

    #[tokio::test]
    async fn mock_adapter_returns_name() {
        let adapter = MockAdapter::new("test-provider");
        assert_eq!(adapter.name(), "test-provider");
    }

    #[tokio::test]
    async fn mock_adapter_returns_capabilities() {
        let adapter = MockAdapter::new("test");
        let caps = adapter.capabilities();
        assert!(!caps.streaming);
        assert_eq!(caps.max_context_tokens, 0);
    }

    #[tokio::test]
    async fn mock_adapter_complete_returns_response() {
        let adapter = MockAdapter::new("test");
        let request = NormalizedRequest {
            model_alias: "test-model".to_string(),
            messages: vec![],
            system: None,
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            stream: false,
            metadata: None,
            thinking: None,
            extra: std::collections::HashMap::new(),
        };
        let response = adapter.complete(request).await.unwrap();
        assert_eq!(response.id, "test-id");
        assert_eq!(response.model, "test-model");
        assert_eq!(response.content, "hello");
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(response.usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn mock_adapter_stream_returns_events() {
        let adapter = MockAdapter::new("test");
        let request = NormalizedRequest {
            model_alias: "test-model".to_string(),
            messages: vec![],
            system: None,
            tools: vec![],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            stream: false,
            metadata: None,
            thinking: None,
            extra: std::collections::HashMap::new(),
        };
        let mut stream = adapter.stream(request).await.unwrap();
        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::MessageStop { .. }));
    }

    #[tokio::test]
    async fn mock_adapter_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockAdapter>();
    }
}
