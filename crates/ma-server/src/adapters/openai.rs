use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use ma_core::adapter::{
    NormalizedResponse, ProviderAdapter, ProviderCapabilities, StopReason, Usage,
};
use ma_core::normalized::{
    ContentDelta, NormalizedContent, NormalizedEvent, NormalizedMessage, NormalizedRequest,
};
use ma_core::{ErrorCategory, NormalizedError};
use serde::Deserialize;

fn openai_capabilities() -> ProviderCapabilities {
    ProviderCapabilities::new(true, true, true, true, false, 128_000, 16_384)
}

#[derive(Debug, Clone)]
pub struct OpenAiAdapter {
    name: String,
    base_url: String,
    api_key: Option<String>,
    upstream_model: String,
    client: reqwest::Client,
}

impl OpenAiAdapter {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        upstream_model: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key,
            upstream_model: upstream_model.into(),
            client: reqwest::Client::new(),
        }
    }

    fn build_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    fn build_request_body(&self, request: &NormalizedRequest) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.upstream_model,
            "messages": request.messages.iter().map(openai_message).collect::<Vec<_>>(),
        });

        if let Some(system) = &request.system {
            let system_text = match system {
                NormalizedContent::Text { text } => text.clone(),
                _ => serde_json::to_string(system).unwrap_or_default(),
            };
            body["messages"] = serde_json::json!(
                [
                    vec![serde_json::json!({"role": "system", "content": system_text})],
                    body["messages"].as_array().cloned().unwrap_or_default()
                ]
                .concat()
            );
        }

        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = max_tokens.into();
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = temperature.into();
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::to_value(&request.tools).unwrap_or_default();
        }
        if let Some(tool_choice) = &request.tool_choice {
            body["tool_choice"] = match tool_choice {
                ma_core::normalized::ToolChoice::Auto => serde_json::json!("auto"),
                ma_core::normalized::ToolChoice::Any => serde_json::json!("any"),
                ma_core::normalized::ToolChoice::None => serde_json::json!("none"),
                ma_core::normalized::ToolChoice::Specific { name } => {
                    serde_json::json!({"type": "function", "function": {"name": name}})
                }
                _ => serde_json::json!("auto"),
            };
        }
        if request.stream {
            body["stream"] = true.into();
        }

        for (key, value) in &request.extra {
            if key != "model" {
                body[key] = value.clone();
            }
        }

        body
    }

    fn strip_thinking_from_request(request: &NormalizedRequest) -> NormalizedRequest {
        let mut req = request.clone();
        req.thinking = None;
        req
    }

    fn classify_http_error(&self, status: reqwest::StatusCode, body: &str) -> NormalizedError {
        let retryable = matches!(
            status,
            reqwest::StatusCode::TOO_MANY_REQUESTS
                | reqwest::StatusCode::REQUEST_TIMEOUT
                | reqwest::StatusCode::BAD_GATEWAY
                | reqwest::StatusCode::SERVICE_UNAVAILABLE
                | reqwest::StatusCode::GATEWAY_TIMEOUT
        ) || status.is_server_error();

        let category = match status {
            reqwest::StatusCode::UNAUTHORIZED => ErrorCategory::Auth,
            reqwest::StatusCode::TOO_MANY_REQUESTS => ErrorCategory::RateLimited,
            reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::GATEWAY_TIMEOUT => {
                ErrorCategory::Timeout
            }
            reqwest::StatusCode::BAD_GATEWAY | reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                ErrorCategory::Overloaded
            }
            status if status.is_server_error() => ErrorCategory::Overloaded,
            status if status.is_client_error() => ErrorCategory::InvalidRequest,
            _ => ErrorCategory::Unknown,
        };

        NormalizedError {
            category,
            retryable,
            http_status: status.as_u16(),
            provider_code: None,
            safe_message: format!("OpenAI upstream returned {status}"),
            raw_debug: Some(body.to_string()),
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        openai_capabilities()
    }

    async fn complete(
        &self,
        request: NormalizedRequest,
    ) -> Result<NormalizedResponse, NormalizedError> {
        let url = self.build_url();
        let request = Self::strip_thinking_from_request(&request);
        let body = self.build_request_body(&request);

        let mut req = self.client.post(&url).json(&body);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }

        let response = req.send().await.map_err(|e| NormalizedError {
            category: if e.is_timeout() {
                ErrorCategory::Timeout
            } else {
                ErrorCategory::Network
            },
            retryable: true,
            http_status: 502,
            provider_code: None,
            safe_message: "upstream request failed".to_string(),
            raw_debug: Some(e.to_string()),
        })?;

        let status = response.status();
        let text = response.text().await.map_err(|e| NormalizedError {
            category: ErrorCategory::Network,
            retryable: true,
            http_status: 502,
            provider_code: None,
            safe_message: "failed to read upstream response".to_string(),
            raw_debug: Some(e.to_string()),
        })?;

        if !status.is_success() {
            return Err(self.classify_http_error(status, &text));
        }

        let completion: OpenAiCompletion =
            serde_json::from_str(&text).map_err(|e| NormalizedError {
                category: ErrorCategory::ProviderBug,
                retryable: false,
                http_status: 502,
                provider_code: None,
                safe_message: "failed to parse upstream response".to_string(),
                raw_debug: Some(e.to_string()),
            })?;

        let choice = completion
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| NormalizedError {
                category: ErrorCategory::ProviderBug,
                retryable: false,
                http_status: 502,
                provider_code: None,
                safe_message: "upstream response had no choices".to_string(),
                raw_debug: None,
            })?;

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::Stop,
            Some("length") => StopReason::Length,
            Some("tool_calls") => StopReason::ToolCall,
            Some("content_filter") => StopReason::ContentFilter,
            _ => StopReason::Stop,
        };

        let usage = completion
            .usage
            .map(|u| Usage::new(u.prompt_tokens, u.completion_tokens, u.total_tokens))
            .unwrap_or(Usage::new(0, 0, 0));

        Ok(NormalizedResponse::new(
            completion.id,
            completion.model,
            choice.message.content.unwrap_or_default(),
            stop_reason,
            usage,
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
        let request = Self::strip_thinking_from_request(&request);
        let mut body = self.build_request_body(&request);
        body["stream"] = true.into();

        let mut req = self.client.post(&url).json(&body);
        if let Some(api_key) = &self.api_key {
            req = req.bearer_auth(api_key);
        }

        let response = req.send().await.map_err(|e| NormalizedError {
            category: if e.is_timeout() {
                ErrorCategory::Timeout
            } else {
                ErrorCategory::Network
            },
            retryable: true,
            http_status: 502,
            provider_code: None,
            safe_message: "upstream request failed".to_string(),
            raw_debug: Some(e.to_string()),
        })?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(self.classify_http_error(status, &text));
        }

        let upstream_model = self.upstream_model.clone();
        let stream = response.bytes_stream().flat_map(move |chunk| {
            let upstream_model = upstream_model.clone();
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let events = parse_sse_events(&text, &upstream_model);
                    futures_util::stream::iter(events)
                }
                Err(e) => futures_util::stream::iter(vec![Err(NormalizedError {
                    category: ErrorCategory::Network,
                    retryable: true,
                    http_status: 502,
                    provider_code: None,
                    safe_message: "stream read error".to_string(),
                    raw_debug: Some(e.to_string()),
                })]),
            }
        });

        Ok(Box::pin(stream))
    }
}

fn openai_message(msg: &NormalizedMessage) -> serde_json::Value {
    let content = match &msg.content {
        NormalizedContent::Text { text } => serde_json::json!(text),
        NormalizedContent::Image { source, .. } => {
            serde_json::json!([{"type": "image_url", "image_url": {"url": source.url.as_deref().unwrap_or("")}}])
        }
        NormalizedContent::ToolUse {
            id, name, input, ..
        } => {
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": input.to_string()
                    }
                }]
            })
        }
        NormalizedContent::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let content_str = match content.as_ref() {
                NormalizedContent::Text { text } => text.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_use_id,
                "content": content_str
            })
        }
        NormalizedContent::Thinking { thinking, .. } => serde_json::json!(thinking),
        NormalizedContent::Mixed { items } => {
            serde_json::json!(items.iter().map(|item| {
                match item {
                    NormalizedContent::Text { text } => serde_json::json!({"type": "text", "text": text}),
                    NormalizedContent::Image { source, .. } => {
                        serde_json::json!({"type": "image_url", "image_url": {"url": source.url.as_deref().unwrap_or("")}})
                    }
                    _ => serde_json::json!({"type": "text", "text": ""})
                }
            }).collect::<Vec<_>>())
        }
        _ => serde_json::json!(""),
    };

    let role = match msg.role {
        ma_core::normalized::MessageRole::System => "system",
        ma_core::normalized::MessageRole::User => "user",
        ma_core::normalized::MessageRole::Assistant => "assistant",
        ma_core::normalized::MessageRole::Tool => "tool",
        _ => "user",
    };

    serde_json::json!({"role": role, "content": content})
}

#[derive(Debug, Deserialize)]
struct OpenAiCompletion {
    id: String,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    id: Option<String>,
    model: Option<String>,
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiStreamDelta {
    content: Option<String>,
    #[expect(dead_code)]
    role: Option<String>,
}

fn parse_sse_events(
    text: &str,
    upstream_model: &str,
) -> Vec<Result<NormalizedEvent, NormalizedError>> {
    let mut events = Vec::new();
    let mut message_started = false;
    let mut message_stopped = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(":") {
            continue;
        }
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            if !message_stopped {
                events.push(Ok(NormalizedEvent::MessageStop {
                    extra: HashMap::new(),
                }));
                message_stopped = true;
            }
            continue;
        }

        let chunk: OpenAiStreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                events.push(Err(NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 502,
                    provider_code: None,
                    safe_message: "failed to parse SSE chunk".to_string(),
                    raw_debug: Some(e.to_string()),
                }));
                continue;
            }
        };

        let id = chunk.id.unwrap_or_default();
        let model = chunk.model.as_deref().unwrap_or(upstream_model);

        if !message_started && !id.is_empty() {
            events.push(Ok(NormalizedEvent::MessageStart {
                id: id.clone(),
                model: model.to_string(),
                extra: HashMap::new(),
            }));
            events.push(Ok(NormalizedEvent::ContentBlockStart {
                index: 0,
                block: NormalizedContent::Text {
                    text: "".to_string(),
                },
                extra: HashMap::new(),
            }));
            message_started = true;
        }

        for choice in chunk.choices {
            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                events.push(Ok(NormalizedEvent::ContentBlockDelta {
                    index: 0,
                    delta: ContentDelta::TextDelta { text: content },
                    extra: HashMap::new(),
                }));
            }

            if let Some(finish_reason) = choice.finish_reason {
                events.push(Ok(NormalizedEvent::ContentBlockStop {
                    index: 0,
                    extra: HashMap::new(),
                }));
                let stop_reason = match finish_reason.as_str() {
                    "stop" => Some("end_turn".to_string()),
                    "length" => Some("max_tokens".to_string()),
                    "tool_calls" => Some("tool_use".to_string()),
                    "content_filter" => Some("content_filter".to_string()),
                    _ => Some(finish_reason),
                };
                events.push(Ok(NormalizedEvent::MessageDelta {
                    stop_reason,
                    extra: HashMap::new(),
                }));
                if !message_stopped {
                    events.push(Ok(NormalizedEvent::MessageStop {
                        extra: HashMap::new(),
                    }));
                    message_stopped = true;
                }
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::spawn_test_server;
    use axum::{Json, Router, routing::post};
    use futures_util::StreamExt;
    use serde_json::{Value, json};

    fn make_request() -> NormalizedRequest {
        NormalizedRequest::new("assemble-main".to_string())
    }

    #[test]
    fn openai_adapter_name_is_correct() {
        let adapter = OpenAiAdapter::new("test-openai", "http://localhost:1234/v1", None, "gpt-4o");
        assert_eq!(adapter.name(), "test-openai");
    }

    #[test]
    fn openai_adapter_capabilities() {
        let adapter = OpenAiAdapter::new("test-openai", "http://localhost:1234/v1", None, "gpt-4o");
        let caps = adapter.capabilities();
        assert!(caps.streaming);
        assert!(caps.tools);
        assert!(caps.parallel_tool_calls);
        assert!(caps.vision);
        assert!(!caps.thinking);
    }

    #[tokio::test]
    async fn openai_adapter_complete_returns_expected_response() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "real-upstream-model");
                Json(json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "model": "real-upstream-model",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "Hello from OpenAI"
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 5,
                        "total_tokens": 15
                    }
                }))
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new(
            "test-openai",
            format!("http://{upstream_addr}"),
            None,
            "real-upstream-model",
        );

        let request = make_request();
        let response = adapter.complete(request).await.unwrap();

        assert_eq!(response.id, "chatcmpl-test");
        assert_eq!(response.model, "real-upstream-model");
        assert_eq!(response.content, "Hello from OpenAI");
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(response.usage.prompt_tokens, 10);
        assert_eq!(response.usage.completion_tokens, 5);
        assert_eq!(response.usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn openai_adapter_complete_rewrites_model_alias() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "gpt-4o-real");
                Json(json!({
                    "id": "test",
                    "model": "gpt-4o-real",
                    "choices": [{
                        "message": { "content": "ok" },
                        "finish_reason": "stop"
                    }]
                }))
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new(
            "test",
            format!("http://{upstream_addr}"),
            None,
            "gpt-4o-real",
        );

        let mut request = make_request();
        request.model_alias = "assemble-main".to_string();
        let response = adapter.complete(request).await.unwrap();
        assert_eq!(response.model, "gpt-4o-real");
    }

    #[tokio::test]
    async fn openai_adapter_complete_uses_bearer_auth() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(
                |headers: http::HeaderMap, Json(_body): Json<Value>| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert!(auth.starts_with("Bearer "));
                    assert!(auth.contains("test-api-key"));
                    Json(json!({
                        "id": "test",
                        "model": "gpt-4",
                        "choices": [{
                            "message": { "content": "authed" },
                            "finish_reason": "stop"
                        }]
                    }))
                },
            ),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new(
            "test",
            format!("http://{upstream_addr}"),
            Some("test-api-key".to_string()),
            "gpt-4",
        );

        let response = adapter.complete(make_request()).await.unwrap();
        assert_eq!(response.content, "authed");
    }

    #[tokio::test]
    async fn openai_adapter_complete_classifies_429_as_retryable() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|| async move {
                (
                    http::StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error": {"message": "rate limited"}})),
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new("test", format!("http://{upstream_addr}"), None, "gpt-4");

        let err = adapter.complete(make_request()).await.unwrap_err();
        assert_eq!(err.http_status, 429);
        assert!(err.retryable);
        assert_eq!(err.category, ErrorCategory::RateLimited);
    }

    #[tokio::test]
    async fn openai_adapter_complete_classifies_401_as_non_retryable() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|| async move {
                (
                    http::StatusCode::UNAUTHORIZED,
                    Json(json!({"error": {"message": "invalid key"}})),
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new("test", format!("http://{upstream_addr}"), None, "gpt-4");

        let err = adapter.complete(make_request()).await.unwrap_err();
        assert_eq!(err.http_status, 401);
        assert!(!err.retryable);
        assert_eq!(err.category, ErrorCategory::Auth);
    }

    #[tokio::test]
    async fn openai_adapter_stream_returns_expected_events() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|Json(_body): Json<Value>| async move {
                (
                    [(http::header::CONTENT_TYPE, "text/event-stream")],
                    "data: {\"id\":\"chatcmpl-test\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n\
                     data: {\"id\":\"chatcmpl-test\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
                     data: {\"id\":\"chatcmpl-test\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
                     data: {\"id\":\"chatcmpl-test\",\"model\":\"gpt-4\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                     data: [DONE]\n\n",
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new("test", format!("http://{upstream_addr}"), None, "gpt-4");

        let request = make_request();
        let mut stream = adapter.stream(request).await.unwrap();

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::MessageStart { ref id, ref model, .. } if id == "chatcmpl-test" && model == "gpt-4")
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::ContentBlockStart { index: 0, block: NormalizedContent::Text { ref text }, .. } if text.is_empty())
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::ContentBlockDelta { index: 0, delta: ContentDelta::TextDelta { ref text }, .. } if text == "Hello")
        );

        let event = stream.next().await.unwrap().unwrap();
        assert!(
            matches!(event, NormalizedEvent::ContentBlockDelta { index: 0, delta: ContentDelta::TextDelta { ref text }, .. } if text == " world")
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

    #[tokio::test]
    async fn openai_adapter_stream_classifies_error_status() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|| async move {
                (
                    http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"error": {"message": "overloaded"}})),
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new("test", format!("http://{upstream_addr}"), None, "gpt-4");

        let result = adapter.stream(make_request()).await;
        assert!(result.is_err());
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert_eq!(err.http_status, 503);
        assert!(err.retryable);
        assert_eq!(err.category, ErrorCategory::Overloaded);
    }

    #[tokio::test]
    async fn openai_adapter_complete_passes_tools() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert!(body["tools"].is_array());
                assert_eq!(body["tools"].as_array().unwrap().len(), 1);
                assert_eq!(body["tool_choice"], "auto");
                Json(json!({
                    "id": "test",
                    "model": "gpt-4",
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call_123",
                                "type": "function",
                                "function": {
                                    "name": "get_weather",
                                    "arguments": "{\"location\":\"NYC\"}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                }))
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let adapter = OpenAiAdapter::new("test", format!("http://{upstream_addr}"), None, "gpt-4");

        let mut request = make_request();
        let tool: ma_core::normalized::ToolDef = serde_json::from_value(serde_json::json!({
            "name": "get_weather",
            "description": "Get weather",
            "input_schema": {"type": "object"}
        }))
        .unwrap();
        request.tools = vec![tool];
        request.tool_choice = Some(ma_core::normalized::ToolChoice::Auto);

        let response = adapter.complete(request).await.unwrap();
        assert_eq!(response.stop_reason, StopReason::ToolCall);
    }
}
