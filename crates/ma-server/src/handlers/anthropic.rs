use axum::body::Body;
use axum::response::Response;
use axum::{Json, extract::State, response::IntoResponse};
use futures_util::{StreamExt, TryStreamExt};
use http::{HeaderMap, StatusCode};
use ma_core::ProviderType;
use ma_core::normalized::NormalizedRequest;
use serde_json::Value;

use crate::{
    AppState, auth::unauthorized_if_needed, error::error_response, fallback::stream_with_fallback,
};

pub async fn anthropic_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if let Some(response) = unauthorized_if_needed(&state, &headers) {
        return response;
    }

    handle_anthropic_normalized(State(state), headers, Json(request)).await
}

pub async fn handle_anthropic_normalized(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let model_alias = match request.get("model").and_then(Value::as_str) {
        Some(model) => model.to_string(),
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "request must include a string model",
            );
        }
    };

    let route = match state.config.models.get(&model_alias) {
        Some(r) => r,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("unknown model alias `{model_alias}`"),
            );
        }
    };

    let provider = match state.config.providers.get(&route.provider) {
        Some(provider) => provider,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("model alias `{model_alias}` references unknown provider"),
            );
        }
    };

    if provider.provider_type == ProviderType::AnthropicCompatible {
        let provider_name = route.provider.clone();
        let upstream_model = route.model.clone();
        return proxy_anthropic_native(state, headers, request, &provider_name, &upstream_model)
            .await;
    }

    let normalized_request: NormalizedRequest = match serde_json::from_value(request) {
        Ok(req) => req,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("failed to parse request: {e}"),
            );
        }
    };

    let stream = normalized_request.stream;

    let adapter = match state.adapter_registry.get(&route.provider) {
        Some(a) => a.clone(),
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("model alias `{model_alias}` references unknown provider"),
            );
        }
    };

    if stream {
        handle_anthropic_normalized_stream(state, normalized_request, model_alias).await
    } else {
        handle_anthropic_normalized_complete(state, adapter, normalized_request, model_alias).await
    }
}

async fn proxy_anthropic_native(
    state: AppState,
    headers: HeaderMap,
    mut request: Value,
    provider_name: &str,
    upstream_model: &str,
) -> Response {
    let Some(provider) = state.config.providers.get(provider_name) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("unknown provider `{provider_name}`"),
        );
    };
    let Some(base_url) = provider.base_url.as_deref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("provider `{provider_name}` is missing base_url"),
        );
    };

    request["model"] = serde_json::json!(upstream_model);
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let api_key = provider
        .api_key_env
        .as_ref()
        .and_then(|env_name| std::env::var(env_name).ok());

    let upstream_headers = build_anthropic_proxy_headers(&headers, api_key.as_deref());

    let upstream_response = match state
        .http
        .post(url)
        .headers(upstream_headers)
        .json(&request)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "network",
                format!("upstream request failed: {error}"),
            );
        }
    };

    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let fallback_content_type = if request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "text/event-stream"
    } else {
        "application/json"
    };

    let stream = upstream_response
        .bytes_stream()
        .map_err(std::io::Error::other);

    let mut builder = Response::builder().status(status);
    copy_anthropic_response_headers(builder.headers_mut().unwrap(), &response_headers);
    if !builder
        .headers_ref()
        .is_some_and(|headers| headers.contains_key(http::header::CONTENT_TYPE))
    {
        builder = builder.header(http::header::CONTENT_TYPE, fallback_content_type);
    }

    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("failed to build upstream response: {error}"),
            )
        })
}

fn build_anthropic_proxy_headers(
    incoming: &HeaderMap,
    api_key: Option<&str>,
) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();

    for (name, value) in incoming {
        if should_forward_anthropic_request_header(name.as_str())
            && let Ok(header_name) =
                reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
        {
            headers.insert(header_name, value.clone());
        }
    }

    if !headers.contains_key("anthropic-version") {
        headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
    }

    if let Some(key) = api_key {
        if incoming.contains_key(http::header::AUTHORIZATION) {
            if let Ok(value) = format!("Bearer {key}").parse() {
                headers.insert(http::header::AUTHORIZATION, value);
            }
            headers.remove("x-api-key");
        } else if let Ok(value) = key.parse() {
            headers.insert("x-api-key", value);
        }
    }

    headers
}

fn should_forward_anthropic_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "host"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "content-encoding"
            | "authorization"
            | "x-api-key"
            | "cookie"
    ) {
        return false;
    }
    lower == "accept"
        || lower == "user-agent"
        || lower == "content-type"
        || lower.starts_with("anthropic-")
        || lower.starts_with("x-")
        || lower.starts_with("claude-")
}

fn copy_anthropic_response_headers(target: &mut HeaderMap, upstream: &HeaderMap) {
    for (name, value) in upstream {
        if should_forward_anthropic_response_header(name.as_str()) {
            target.insert(name.clone(), value.clone());
        }
    }
}

fn should_forward_anthropic_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "connection"
            | "content-length"
            | "transfer-encoding"
            | "content-encoding"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "upgrade"
    )
}

async fn handle_anthropic_normalized_complete(
    state: AppState,
    adapter: std::sync::Arc<dyn ma_core::adapter::ProviderAdapter>,
    mut request: NormalizedRequest,
    model_alias: String,
) -> Response {
    let route = match state.config.models.get(&model_alias) {
        Some(r) => r,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                format!("unknown model alias `{model_alias}`"),
            );
        }
    };
    request.model_alias = route.model.clone();

    match adapter.complete(request).await {
        Ok(response) => {
            let content_blocks: Vec<serde_json::Value> = vec![serde_json::json!({
                "type": "text",
                "text": response.content
            })];

            let anthropic_response = serde_json::json!({
                "id": response.id,
                "type": "message",
                "role": "assistant",
                "model": response.model,
                "content": content_blocks,
                "stop_reason": match response.stop_reason {
                    ma_core::adapter::StopReason::Stop => "end_turn",
                    ma_core::adapter::StopReason::Length => "max_tokens",
                    ma_core::adapter::StopReason::ToolCall => "tool_use",
                    ma_core::adapter::StopReason::ContentFilter => "content_filter",
                    _ => "end_turn",
                },
                "usage": {
                    "input_tokens": response.usage.prompt_tokens,
                    "output_tokens": response.usage.completion_tokens
                }
            });
            Json(anthropic_response).into_response()
        }
        Err(e) => {
            let category = format!("{:?}", e.category);
            error_response(
                StatusCode::from_u16(e.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                &category,
                &e.safe_message,
            )
        }
    }
}

async fn handle_anthropic_normalized_stream(
    state: AppState,
    request: NormalizedRequest,
    model_alias: String,
) -> Response {
    let mut stream = match stream_with_fallback(&state, request, &model_alias).await {
        Ok(s) => s,
        Err(e) => {
            let category = format!("{:?}", e.category);
            return error_response(
                StatusCode::from_u16(e.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                &category,
                &e.safe_message,
            );
        }
    };

    let sse_stream = async_stream::stream! {
        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => {
                    let (event_type, data) = match event {
                        ma_core::normalized::NormalizedEvent::MessageStart { id, model, .. } => {
                            ("message_start", serde_json::json!({
                                "type": "message_start",
                                "message": {
                                    "id": id,
                                    "type": "message",
                                    "role": "assistant",
                                    "model": model,
                                    "content": [],
                                    "stop_reason": null,
                                    "usage": null
                                }
                            }))
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockStart { index, block, .. } => {
                            let content_block = match block {
                                ma_core::normalized::NormalizedContent::Text { text } => {
                                    serde_json::json!({"type": "text", "text": text})
                                }
                                ma_core::normalized::NormalizedContent::ToolUse { id, name, input, .. } => {
                                    serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input})
                                }
                                ma_core::normalized::NormalizedContent::Thinking { thinking, signature, .. } => {
                                    let mut obj = serde_json::json!({"type": "thinking", "thinking": thinking});
                                    if let Some(sig) = signature {
                                        obj["signature"] = serde_json::json!(sig);
                                    }
                                    obj
                                }
                                _ => serde_json::json!({"type": "text", "text": ""})
                            };
                            ("content_block_start", serde_json::json!({
                                "type": "content_block_start",
                                "index": index,
                                "content_block": content_block
                            }))
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockDelta { index, delta, .. } => {
                            let delta_obj = match delta {
                                ma_core::normalized::ContentDelta::TextDelta { text } => {
                                    serde_json::json!({"type": "text_delta", "text": text})
                                }
                                ma_core::normalized::ContentDelta::ThinkingDelta { thinking } => {
                                    serde_json::json!({"type": "thinking_delta", "thinking": thinking})
                                }
                                ma_core::normalized::ContentDelta::SignatureDelta { signature } => {
                                    serde_json::json!({"type": "signature_delta", "signature": signature})
                                }
                                ma_core::normalized::ContentDelta::InputJsonDelta { partial_json, .. } => {
                                    serde_json::json!({"type": "input_json_delta", "partial_json": partial_json})
                                }
                                _ => serde_json::json!({"type": "text_delta", "text": ""})
                            };
                            ("content_block_delta", serde_json::json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": delta_obj
                            }))
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockStop { index, .. } => {
                            ("content_block_stop", serde_json::json!({
                                "type": "content_block_stop",
                                "index": index
                            }))
                        }
                        ma_core::normalized::NormalizedEvent::MessageDelta { stop_reason, .. } => {
                            let mut delta = serde_json::json!({});
                            if let Some(reason) = stop_reason {
                                delta["stop_reason"] = serde_json::json!(reason);
                            }
                            ("message_delta", serde_json::json!({
                                "type": "message_delta",
                                "delta": delta
                            }))
                        }
                        ma_core::normalized::NormalizedEvent::MessageStop { .. } => {
                            ("message_stop", serde_json::json!({"type": "message_stop"}))
                        }
                        ma_core::normalized::NormalizedEvent::Error { error, .. } => {
                            ("error", serde_json::json!({
                                "type": "error",
                                "error": {
                                    "type": format!("{:?}", error.category),
                                    "message": error.safe_message
                                }
                            }))
                        }
                        _ => {
                            ("message_stop", serde_json::json!({"type": "message_stop"}))
                        }
                    };
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(
                        format!("event: {}\ndata: {}\n\n", event_type, data)
                    ));
                }
                Err(e) => {
                    let error_json = serde_json::json!({
                        "type": "error",
                        "error": {
                            "type": format!("{:?}", e.category),
                            "message": e.safe_message
                        }
                    });
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(
                        format!("event: error\ndata: {}\n\n", error_json)
                    ));
                    break;
                }
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(sse_stream))
        .unwrap_or_else(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("failed to build stream response: {e}"),
            )
        })
}
