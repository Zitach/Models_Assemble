use axum::body::Body;
use axum::response::Response;
use axum::{Json, extract::State, response::IntoResponse};
use futures_util::StreamExt;
use http::{HeaderMap, StatusCode};
use ma_core::AppConfig;
use ma_core::normalized::NormalizedRequest;
use serde_json::Value;
use std::net::SocketAddr;

use crate::{AppState, auth::unauthorized_if_needed, error::error_response};

pub async fn openai_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if let Some(response) = unauthorized_if_needed(&state, &headers) {
        return response;
    }

    handle_openai_normalized(State(state), headers, Json(request)).await
}

pub async fn handle_openai_normalized(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
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

    let model_alias = normalized_request.model_alias.clone();
    let stream = normalized_request.stream;

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
        handle_openai_normalized_stream(state, adapter, normalized_request, model_alias).await
    } else {
        handle_openai_normalized_complete(state, adapter, normalized_request, model_alias).await
    }
}

async fn handle_openai_normalized_complete(
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
            let openai_response = serde_json::json!({
                "id": response.id,
                "object": "chat.completion",
                "created": 0,
                "model": response.model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": response.content
                    },
                    "finish_reason": match response.stop_reason {
                        ma_core::adapter::StopReason::Stop => "stop",
                        ma_core::adapter::StopReason::Length => "length",
                        ma_core::adapter::StopReason::ToolCall => "tool_calls",
                        ma_core::adapter::StopReason::ContentFilter => "content_filter",
                        _ => "stop",
                    }
                }],
                "usage": {
                    "prompt_tokens": response.usage.prompt_tokens,
                    "completion_tokens": response.usage.completion_tokens,
                    "total_tokens": response.usage.total_tokens
                }
            });
            Json(openai_response).into_response()
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

async fn handle_openai_normalized_stream(
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

    let mut stream = match adapter.stream(request).await {
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
                    let sse_data = match event {
                        ma_core::normalized::NormalizedEvent::MessageStart { id, model, .. } => {
                            serde_json::json!({
                                "id": id,
                                "object": "chat.completion.chunk",
                                "model": model,
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": null
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockStart { .. } => {
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": null
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockDelta { delta, .. } => {
                            let content = match delta {
                                ma_core::normalized::ContentDelta::TextDelta { text } => text,
                                _ => String::new(),
                            };
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {
                                        "content": content
                                    },
                                    "finish_reason": null
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::ContentBlockStop { .. } => {
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": null
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::MessageDelta { stop_reason, .. } => {
                            let finish_reason = stop_reason.as_deref();
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": finish_reason
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::MessageStop { .. } => {
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": "stop"
                                }]
                            })
                        }
                        ma_core::normalized::NormalizedEvent::Error { error, .. } => {
                            serde_json::json!({
                                "error": {
                                    "message": error.safe_message,
                                    "type": format!("{:?}", error.category)
                                }
                            })
                        }
                        _ => {
                            serde_json::json!({
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "finish_reason": null
                                }]
                            })
                        }
                    };
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(
                        format!("data: {}\n\n", sse_data)
                    ));
                }
                Err(e) => {
                    let error_json = serde_json::json!({
                        "error": {
                            "message": e.safe_message,
                            "type": format!("{:?}", e.category)
                        }
                    });
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(
                        format!("data: {}\n\n", error_json)
                    ));
                    break;
                }
            }
        }
        yield Ok::<_, std::io::Error>(axum::body::Bytes::from("data: [DONE]\n\n"));
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

pub fn compat_config(bind: SocketAddr) -> AppConfig {
    AppConfig {
        server: ma_core::ServerConfig {
            bind,
            api_keys: Vec::new(),
            first_token_timeout_secs: None,
        },
        ..AppConfig::default()
    }
}
