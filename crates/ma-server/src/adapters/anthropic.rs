use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use ma_core::adapter::{
    NormalizedResponse, ProviderAdapter, ProviderCapabilities, StopReason, Usage,
};
use ma_core::normalized::{ContentDelta, NormalizedContent, NormalizedEvent, NormalizedRequest};
use ma_core::{ErrorCategory, NormalizedError};
use serde::Deserialize;

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicAdapter {
    name: String,
    base_url: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl AnthropicAdapter {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key,
            model: model.into(),
            client: reqwest::Client::new(),
        }
    }

    fn build_url(&self) -> String {
        format!("{}/messages", self.base_url.trim_end_matches('/'))
    }

    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("anthropic-version", ANTHROPIC_VERSION.parse().unwrap());
        if let Some(ref key) = self.api_key {
            headers.insert("x-api-key", key.parse().unwrap());
        }
        headers
    }

    fn request_to_body(&self, request: NormalizedRequest) -> serde_json::Value {
        let model = if self.model.is_empty() {
            request.model_alias.clone()
        } else {
            self.model.clone()
        };
        let mut body = serde_json::json!({
            "model": model,
            "messages": request.messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });
        if let Some(system) = request.system {
            body["system"] = serde_json::to_value(system).unwrap_or_default();
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::to_value(request.tools).unwrap_or_default();
        }
        if let Some(tool_choice) = request.tool_choice {
            body["tool_choice"] = serde_json::to_value(tool_choice).unwrap_or_default();
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }
        if let Some(thinking) = request.thinking {
            body["thinking"] = serde_json::to_value(thinking).unwrap_or_default();
        }
        if request.stream {
            body["stream"] = serde_json::json!(true);
        }
        for (k, v) in request.extra {
            body[k] = v;
        }
        body
    }

    fn map_stop_reason(reason: Option<String>) -> StopReason {
        match reason.as_deref() {
            Some("end_turn") => StopReason::Stop,
            Some("max_tokens") => StopReason::Length,
            Some("tool_use") => StopReason::ToolCall,
            Some("content_filter") => StopReason::ContentFilter,
            _ => StopReason::Stop,
        }
    }

    fn map_content_block(block: serde_json::Value) -> Option<NormalizedContent> {
        let block_type = block.get("type")?.as_str()?;
        match block_type {
            "text" => {
                let text = block.get("text")?.as_str()?.to_string();
                Some(NormalizedContent::Text { text })
            }
            "tool_use" => {
                let id = block.get("id")?.as_str()?.to_string();
                let name = block.get("name")?.as_str()?.to_string();
                let input = block.get("input").cloned().unwrap_or_default();
                Some(NormalizedContent::ToolUse {
                    id,
                    name,
                    input,
                    extra: HashMap::new(),
                })
            }
            "thinking" => {
                let thinking = block.get("thinking")?.as_str()?.to_string();
                let signature = block
                    .get("signature")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Some(NormalizedContent::Thinking {
                    thinking,
                    signature,
                    extra: HashMap::new(),
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageResponse {
    id: String,
    #[serde(rename = "type")]
    _type: String,
    model: String,
    content: Vec<serde_json::Value>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[async_trait]
impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::new(true, true, false, false, true, 200000, 8192)
    }

    async fn complete(
        &self,
        request: NormalizedRequest,
    ) -> Result<NormalizedResponse, NormalizedError> {
        let url = self.build_url();
        let body = self.request_to_body(request);

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| NormalizedError {
                category: ErrorCategory::Network,
                retryable: true,
                http_status: 502,
                provider_code: None,
                safe_message: "upstream request failed".to_string(),
                raw_debug: Some(e.to_string()),
            })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(NormalizedError {
                category: if status.as_u16() == 429 {
                    ErrorCategory::RateLimited
                } else if status.is_server_error() {
                    ErrorCategory::Overloaded
                } else {
                    ErrorCategory::InvalidRequest
                },
                retryable: status.is_server_error() || status.as_u16() == 429,
                http_status: status.as_u16(),
                provider_code: None,
                safe_message: format!("upstream returned status {}", status),
                raw_debug: Some(text),
            });
        }

        let data: AnthropicMessageResponse =
            response.json().await.map_err(|e| NormalizedError {
                category: ErrorCategory::ProviderBug,
                retryable: false,
                http_status: 502,
                provider_code: None,
                safe_message: "failed to parse upstream response".to_string(),
                raw_debug: Some(e.to_string()),
            })?;

        let content = data
            .content
            .into_iter()
            .filter_map(|block| {
                let block_type = block.get("type")?.as_str()?;
                match block_type {
                    "text" => Some(block.get("text")?.as_str()?.to_string()),
                    "tool_use" => Some(serde_json::to_string(&block).unwrap_or_default()),
                    "thinking" => Some(serde_json::to_string(&block).unwrap_or_default()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join("");

        let usage = data.usage.unwrap_or(AnthropicUsage {
            input_tokens: 0,
            output_tokens: 0,
        });

        Ok(NormalizedResponse::new(
            data.id,
            data.model,
            content,
            Self::map_stop_reason(data.stop_reason),
            Usage::new(
                usage.input_tokens,
                usage.output_tokens,
                usage.input_tokens + usage.output_tokens,
            ),
        ))
    }

    async fn stream(
        &self,
        request: NormalizedRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<NormalizedEvent, NormalizedError>> + Send>>,
        NormalizedError,
    > {
        let url = self.build_url();
        let mut body = self.request_to_body(request);
        body["stream"] = serde_json::json!(true);

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| NormalizedError {
                category: ErrorCategory::Network,
                retryable: true,
                http_status: 502,
                provider_code: None,
                safe_message: "upstream request failed".to_string(),
                raw_debug: Some(e.to_string()),
            })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(NormalizedError {
                category: if status.as_u16() == 429 {
                    ErrorCategory::RateLimited
                } else if status.is_server_error() {
                    ErrorCategory::Overloaded
                } else {
                    ErrorCategory::InvalidRequest
                },
                retryable: status.is_server_error() || status.as_u16() == 429,
                http_status: status.as_u16(),
                provider_code: None,
                safe_message: format!("upstream returned status {}", status),
                raw_debug: Some(text),
            });
        }

        let byte_stream = response.bytes_stream();
        let event_stream = futures_util::stream::unfold(byte_stream, |mut stream| async move {
            match stream.next().await {
                Some(Ok(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let events: Vec<Result<NormalizedEvent, NormalizedError>> =
                        parse_sse_events(&text).into_iter().map(Ok).collect();
                    Some((events, stream))
                }
                Some(Err(e)) => {
                    let err = NormalizedError {
                        category: ErrorCategory::Network,
                        retryable: true,
                        http_status: 502,
                        provider_code: None,
                        safe_message: "stream read error".to_string(),
                        raw_debug: Some(e.to_string()),
                    };
                    Some((vec![Err(err)], stream))
                }
                None => None,
            }
        })
        .flat_map(futures_util::stream::iter);

        Ok(Box::pin(event_stream))
    }
}

fn parse_sse_events(text: &str) -> Vec<NormalizedEvent> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in text.lines() {
        if let Some(stripped) = line.strip_prefix("event: ") {
            current_event = stripped.trim().to_string();
        } else if let Some(stripped) = line.strip_prefix("data: ") {
            current_data = stripped.trim().to_string();
        } else if line.is_empty() && !current_event.is_empty() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&current_data)
                && let Some(event) = map_sse_event(&current_event, json)
            {
                events.push(event);
            }
            current_event.clear();
            current_data.clear();
        }
    }

    if !current_event.is_empty()
        && !current_data.is_empty()
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&current_data)
        && let Some(event) = map_sse_event(&current_event, json)
    {
        events.push(event);
    }

    events
}

fn map_sse_event(event_type: &str, data: serde_json::Value) -> Option<NormalizedEvent> {
    match event_type {
        "message_start" => {
            let message = data.get("message")?;
            let id = message.get("id")?.as_str()?.to_string();
            let model = message.get("model")?.as_str()?.to_string();
            Some(NormalizedEvent::MessageStart {
                id,
                model,
                extra: HashMap::new(),
            })
        }
        "content_block_start" => {
            let index = data.get("index")?.as_u64()? as u32;
            let content_block = data.get("content_block")?.clone();
            let block = AnthropicAdapter::map_content_block(content_block)?;
            Some(NormalizedEvent::ContentBlockStart {
                index,
                block,
                extra: HashMap::new(),
            })
        }
        "content_block_delta" => {
            let index = data.get("index")?.as_u64()? as u32;
            let delta = data.get("delta")?.clone();
            let delta_type = delta.get("type")?.as_str()?;
            let content_delta = match delta_type {
                "text_delta" => {
                    let text = delta.get("text")?.as_str()?.to_string();
                    ContentDelta::TextDelta { text }
                }
                "thinking_delta" => {
                    let thinking = delta.get("thinking")?.as_str()?.to_string();
                    ContentDelta::ThinkingDelta { thinking }
                }
                "signature_delta" => {
                    let signature = delta.get("signature")?.as_str()?.to_string();
                    ContentDelta::SignatureDelta { signature }
                }
                "input_json_delta" => {
                    let partial_json = delta.get("partial_json")?.as_str()?.to_string();
                    ContentDelta::InputJsonDelta {
                        partial_json,
                        extra: HashMap::new(),
                    }
                }
                _ => return None,
            };
            Some(NormalizedEvent::ContentBlockDelta {
                index,
                delta: content_delta,
                extra: HashMap::new(),
            })
        }
        "content_block_stop" => {
            let index = data.get("index")?.as_u64()? as u32;
            Some(NormalizedEvent::ContentBlockStop {
                index,
                extra: HashMap::new(),
            })
        }
        "message_delta" => {
            let delta = data.get("delta")?;
            let stop_reason = delta
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(NormalizedEvent::MessageDelta {
                stop_reason,
                extra: HashMap::new(),
            })
        }
        "message_stop" => Some(NormalizedEvent::MessageStop {
            extra: HashMap::new(),
        }),
        "error" => {
            let error = data.get("error")?;
            let safe_message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            Some(NormalizedEvent::Error {
                error: NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 500,
                    provider_code: None,
                    safe_message,
                    raw_debug: Some(data.to_string()),
                },
                extra: HashMap::new(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;

    #[test]
    fn anthropic_adapter_name() {
        let adapter = AnthropicAdapter::new(
            "test-anthropic",
            "https://api.anthropic.com/v1",
            Some("test-key".to_string()),
            "claude-3-opus",
        );
        assert_eq!(adapter.name(), "test-anthropic");
    }

    #[test]
    fn anthropic_adapter_capabilities() {
        let adapter =
            AnthropicAdapter::new("test", "https://api.anthropic.com/v1", None, "claude-3");
        let caps = adapter.capabilities();
        assert!(caps.streaming);
        assert!(caps.tools);
        assert!(caps.thinking);
        assert_eq!(caps.max_context_tokens, 200000);
    }

    #[test]
    fn request_to_body_maps_fields() {
        let adapter = AnthropicAdapter::new("test", "http://localhost/v1", None, "claude-3");
        let mut request = NormalizedRequest::new("assemble-claude".to_string());
        let msg: ma_core::normalized::NormalizedMessage =
            serde_json::from_value(serde_json::json!({
                "role": "user",
                "content": "hello"
            }))
            .unwrap();
        request.messages = vec![msg];
        request.system = Some(ma_core::normalized::NormalizedContent::Text {
            text: "You are helpful.".to_string(),
        });
        request.max_tokens = Some(1024);
        request.temperature = Some(0.7);

        let body = adapter.request_to_body(request);
        assert_eq!(body["model"], "claude-3");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert!(body["system"].is_object());
    }

    #[test]
    fn request_to_body_uses_request_model_when_adapter_model_is_empty() {
        let adapter = AnthropicAdapter::new("test", "http://localhost/v1", None, "");
        let request = NormalizedRequest::new("glm-5.1".to_string());

        let body = adapter.request_to_body(request);

        assert_eq!(body["model"], "glm-5.1");
    }

    #[test]
    fn map_stop_reason_variants() {
        assert_eq!(
            AnthropicAdapter::map_stop_reason(Some("end_turn".to_string())),
            StopReason::Stop
        );
        assert_eq!(
            AnthropicAdapter::map_stop_reason(Some("max_tokens".to_string())),
            StopReason::Length
        );
        assert_eq!(
            AnthropicAdapter::map_stop_reason(Some("tool_use".to_string())),
            StopReason::ToolCall
        );
        assert_eq!(
            AnthropicAdapter::map_stop_reason(Some("content_filter".to_string())),
            StopReason::ContentFilter
        );
        assert_eq!(AnthropicAdapter::map_stop_reason(None), StopReason::Stop);
    }

    #[test]
    fn map_content_block_text() {
        let block = json!({"type": "text", "text": "hello"});
        let result = AnthropicAdapter::map_content_block(block).unwrap();
        assert_eq!(
            result,
            NormalizedContent::Text {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn map_content_block_tool_use() {
        let block = json!({"type": "tool_use", "id": "tu_1", "name": "calc", "input": {"x": 1}});
        let result = AnthropicAdapter::map_content_block(block).unwrap();
        match result {
            NormalizedContent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "calc");
                assert_eq!(input, json!({"x": 1}));
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn map_content_block_thinking() {
        let block = json!({"type": "thinking", "thinking": "hmm", "signature": "sig"});
        let result = AnthropicAdapter::map_content_block(block).unwrap();
        match result {
            NormalizedContent::Thinking {
                thinking,
                signature,
                ..
            } => {
                assert_eq!(thinking, "hmm");
                assert_eq!(signature, Some("sig".to_string()));
            }
            other => panic!("expected Thinking, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_message_start() {
        let sse = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"claude-3"}}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            NormalizedEvent::MessageStart { id, model, .. } => {
                assert_eq!(id, "msg_1");
                assert_eq!(model, "claude-3");
            }
            other => panic!("expected MessageStart, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_content_block_delta() {
        let sse = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            NormalizedEvent::ContentBlockDelta { index, delta, .. } => {
                assert_eq!(*index, 0);
                assert_eq!(
                    delta,
                    &ContentDelta::TextDelta {
                        text: "hello".to_string()
                    }
                );
            }
            other => panic!("expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_thinking_delta() {
        let sse = r#"event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"let me think"}}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            NormalizedEvent::ContentBlockDelta { delta, .. } => {
                assert_eq!(
                    delta,
                    &ContentDelta::ThinkingDelta {
                        thinking: "let me think".to_string()
                    }
                );
            }
            other => panic!("expected ThinkingDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_input_json_delta() {
        let sse = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"x\": 1"}}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 1);
        match &events[0] {
            NormalizedEvent::ContentBlockDelta { delta, .. } => {
                assert_eq!(
                    delta,
                    &ContentDelta::InputJsonDelta {
                        partial_json: "{\"x\": 1".to_string(),
                        extra: HashMap::new()
                    }
                );
            }
            other => panic!("expected InputJsonDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_sse_message_stop() {
        let sse = r#"event: message_stop
data: {"type":"message_stop"}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], NormalizedEvent::MessageStop { .. }));
    }

    #[test]
    fn parse_sse_multiple_events() {
        let sse = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"claude-3"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}

"#;
        let events = parse_sse_events(sse);
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn complete_with_mock_upstream() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-3",
                    "content": [{"type": "text", "text": "hello world"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 10, "output_tokens": 5}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new(
            "test",
            server.url(),
            Some("test-key".to_string()),
            "claude-3",
        );

        let request = NormalizedRequest::new("assemble-claude".to_string());
        let response = adapter.complete(request).await.unwrap();

        assert_eq!(response.id, "msg_test");
        assert_eq!(response.model, "claude-3");
        assert_eq!(response.content, "hello world");
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.completion_tokens, 5);
        assert_eq!(response.usage.total_tokens, 15);

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn complete_returns_error_on_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/messages")
            .with_status(429)
            .with_body(json!({"error": {"message": "rate limited"}}).to_string())
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new("test", server.url(), None, "claude-3");
        let request = NormalizedRequest::new("assemble-claude".to_string());
        let result = adapter.complete(request).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.http_status, 429);
        assert!(err.retryable);

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn stream_with_mock_upstream() {
        let mut server = mockito::Server::new_async().await;
        let sse_body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"claude-3"}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}

"#;
        let mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_body)
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new("test", server.url(), None, "claude-3");
        let request = NormalizedRequest::new("assemble-claude".to_string());
        let mut stream = adapter.stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::MessageStart { .. }));

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::ContentBlockStart { .. }));

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::ContentBlockDelta { .. }));

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::ContentBlockStop { .. }));

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::MessageDelta { .. }));

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::MessageStop { .. }));

        assert!(stream.next().await.is_none());

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn stream_tool_use_events() {
        let mut server = mockito::Server::new_async().await;
        let sse_body = r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu_1","name":"calc","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#;
        let mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_body)
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new("test", server.url(), None, "claude-3");
        let request = NormalizedRequest::new("assemble-claude".to_string());
        let mut stream = adapter.stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        match event {
            NormalizedEvent::ContentBlockStart { block, .. } => {
                assert!(matches!(block, NormalizedContent::ToolUse { .. }));
            }
            other => panic!("expected ContentBlockStart with ToolUse, got {:?}", other),
        }

        let event = stream.next().await.unwrap().unwrap();
        match event {
            NormalizedEvent::ContentBlockDelta { delta, .. } => {
                assert!(matches!(delta, ContentDelta::InputJsonDelta { .. }));
            }
            other => panic!("expected InputJsonDelta, got {:?}", other),
        }

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn stream_thinking_events() {
        let mut server = mockito::Server::new_async().await;
        let sse_body = "event: content_block_start\n\
        data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\
        \n\
        event: content_block_delta\n\
        data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me think...\"}}\n\
        \n\
        event: content_block_delta\n\
        data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_abc123\"}}\n\
        \n\
        event: content_block_stop\n\
        data: {\"type\":\"content_block_stop\",\"index\":0}\n\
        \n";
        let mock = server
            .mock("POST", "/messages")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(sse_body)
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new("test", server.url(), None, "claude-3");
        let request = NormalizedRequest::new("assemble-claude".to_string());
        let mut stream = adapter.stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        match event {
            NormalizedEvent::ContentBlockStart { block, .. } => {
                assert!(matches!(block, NormalizedContent::Thinking { .. }));
            }
            other => panic!("expected ContentBlockStart with Thinking, got {:?}", other),
        }

        let event = stream.next().await.unwrap().unwrap();
        match event {
            NormalizedEvent::ContentBlockDelta { delta, .. } => {
                assert!(matches!(delta, ContentDelta::ThinkingDelta { .. }));
            }
            other => panic!("expected ThinkingDelta, got {:?}", other),
        }

        let event = stream.next().await.unwrap().unwrap();
        match event {
            NormalizedEvent::ContentBlockDelta { delta, .. } => {
                assert!(matches!(delta, ContentDelta::SignatureDelta { .. }));
            }
            other => panic!("expected SignatureDelta, got {:?}", other),
        }

        let event = stream.next().await.unwrap().unwrap();
        assert!(matches!(event, NormalizedEvent::ContentBlockStop { .. }));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn headers_use_x_api_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/messages")
            .match_header("x-api-key", "secret-key")
            .match_header("anthropic-version", "2023-06-01")
            .with_status(200)
            .with_body(
                json!({
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-3",
                    "content": [{"type": "text", "text": "ok"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 1, "output_tokens": 1}
                })
                .to_string(),
            )
            .create_async()
            .await;

        let adapter = AnthropicAdapter::new(
            "test",
            server.url(),
            Some("secret-key".to_string()),
            "claude-3",
        );
        let request = NormalizedRequest::new("assemble-claude".to_string());
        let _ = adapter.complete(request).await;

        mock.assert_async().await;
    }
}
