use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use http::{HeaderMap, StatusCode, header};
use ma_core::{
    AppConfig, ErrorCategory, ModelInfo, ModelList, NormalizedError, ProviderConfig, ProviderType,
    protocol::HealthResponse,
};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    config: Arc<AppConfig>,
    http: Client,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            http: Client::new(),
        }
    }
}

pub async fn serve(config: AppConfig) -> anyhow::Result<()> {
    let bind = config.server.bind;
    let app = router(config);
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;

    tracing::info!(%bind, "models assemble server listening");
    axum::serve(listener, app).await.context("server failed")
}

pub fn router(config: AppConfig) -> Router {
    let state = AppState::new(config);

    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(openai_chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "models-assemble",
    })
}

async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
    let data = state
        .config
        .models
        .keys()
        .map(|id| ModelInfo {
            id: id.clone(),
            object: "model",
            owned_by: "models-assemble",
        })
        .collect();

    Json(ModelList {
        object: "list",
        data,
    })
}

async fn openai_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    if let Some(response) = unauthorized_if_needed(&state, &headers) {
        return response;
    }

    let Some(model_alias) = request.get("model").and_then(Value::as_str) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request is missing string field `model`",
        );
    };
    let model_alias = model_alias.to_string();
    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if stream {
        handle_openai_alias_once(&state, &model_alias, request).await
    } else {
        handle_openai_with_fallback(&state, &model_alias, request).await
    }
}

async fn handle_openai_with_fallback(
    state: &AppState,
    initial_alias: &str,
    request: Value,
) -> Response {
    let mut aliases = vec![initial_alias.to_string()];
    if let Some(fallbacks) = state.config.fallback.get(initial_alias) {
        aliases.extend(fallbacks.iter().cloned());
    }

    let mut last_error = None;
    for alias in aliases {
        match handle_openai_alias(state, &alias, request.clone()).await {
            Ok(response) => return response,
            Err(error) if error.retryable => {
                tracing::warn!(
                    alias = %alias,
                    category = ?error.category,
                    message = %error.safe_message,
                    "openai alias failed, trying fallback if available"
                );
                last_error = Some(error);
            }
            Err(error) => return normalized_error_response(error),
        }
    }

    normalized_error_response(last_error.unwrap_or_else(|| NormalizedError {
        category: ErrorCategory::Unknown,
        retryable: false,
        http_status: 502,
        provider_code: None,
        safe_message: "all fallback targets failed".to_string(),
        raw_debug: None,
    }))
}

async fn handle_openai_alias_once(state: &AppState, alias: &str, request: Value) -> Response {
    match handle_openai_alias(state, alias, request).await {
        Ok(response) => response,
        Err(error) => normalized_error_response(error),
    }
}

async fn handle_openai_alias(
    state: &AppState,
    alias: &str,
    request: Value,
) -> Result<Response, NormalizedError> {
    let route = state
        .config
        .models
        .get(alias)
        .ok_or_else(|| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("unknown model alias `{alias}`"),
            raw_debug: None,
        })?;

    let provider = state
        .config
        .providers
        .get(&route.provider)
        .ok_or_else(|| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("model alias `{alias}` references unknown provider"),
            raw_debug: None,
        })?;

    if provider.provider_type == ProviderType::OpenAiCompatible {
        proxy_openai_compatible(state, provider, &route.model, request).await
    } else {
        Ok(mock_openai_response(alias.to_string(), request))
    }
}

async fn proxy_openai_compatible(
    state: &AppState,
    provider: &ProviderConfig,
    upstream_model: &str,
    mut request: Value,
) -> Result<Response, NormalizedError> {
    let Some(base_url) = provider.base_url.as_deref() else {
        return Err(NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: "openai-compatible provider is missing base_url".to_string(),
            raw_debug: None,
        });
    };

    let api_key = match provider.api_key_env.as_deref() {
        Some(env_name) => Some(std::env::var(env_name).map_err(|error| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("openai-compatible provider API key env `{env_name}` is not set"),
            raw_debug: Some(error.to_string()),
        })?),
        None => None,
    };

    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    request["model"] = Value::String(upstream_model.to_string());
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut upstream_request = state.http.post(url).json(&request);
    if let Some(api_key) = api_key {
        upstream_request = upstream_request.bearer_auth(api_key);
    }
    let upstream = upstream_request.send().await;

    let upstream = match upstream {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(error = %error, "openai-compatible upstream request failed");
            return Err(classify_reqwest_error(error));
        }
    };

    let status = upstream.status();
    if should_fallback_status(status) && !stream {
        return Err(NormalizedError {
            category: classify_status(status),
            retryable: true,
            http_status: status.as_u16(),
            provider_code: None,
            safe_message: format!("upstream provider returned retryable status {status}"),
            raw_debug: None,
        });
    }

    let response = if stream {
        let content_type = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .cloned()
            .unwrap_or_else(|| header::HeaderValue::from_static("text/event-stream"));
        let body = Body::from_stream(
            upstream
                .bytes_stream()
                .map(|item| item.map_err(std::io::Error::other)),
        );

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .unwrap_or_else(|error| {
                tracing::error!(error = %error, "failed to build streaming response");
                normalized_error_response(NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 500,
                    provider_code: None,
                    safe_message: "failed to build streaming response".to_string(),
                    raw_debug: Some(error.to_string()),
                })
            })
    } else {
        let body = match upstream.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(error = %error, "failed to read upstream response body");
                return Err(NormalizedError {
                    category: ErrorCategory::Network,
                    retryable: true,
                    http_status: 502,
                    provider_code: None,
                    safe_message: "failed to read upstream response body".to_string(),
                    raw_debug: Some(error.to_string()),
                });
            }
        };

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap_or_else(|error| {
                tracing::error!(error = %error, "failed to build upstream response");
                normalized_error_response(NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 500,
                    provider_code: None,
                    safe_message: "failed to build upstream response".to_string(),
                    raw_debug: Some(error.to_string()),
                })
            })
    };

    Ok(response)
}

fn mock_openai_response(model: String, request: Value) -> Response {
    if request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        mock_openai_stream(model)
    } else {
        mock_openai_json(model)
    }
}

fn mock_openai_json(model: String) -> Response {
    Json(json!({
        "id": "chatcmpl-ma-compat",
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Models Assemble compat-probe OK."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        }
    }))
    .into_response()
}

fn mock_openai_stream(model: String) -> Response {
    let chunks = vec![
        json!({
            "id": "chatcmpl-ma-compat",
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant", "content": "Models Assemble " },
                "finish_reason": null
            }]
        }),
        json!({
            "id": "chatcmpl-ma-compat",
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": { "content": "compat-probe OK." },
                "finish_reason": null
            }]
        }),
        json!({
            "id": "chatcmpl-ma-compat",
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }),
    ];

    let events = stream::iter(
        chunks
            .into_iter()
            .map(|chunk| Ok::<_, Infallible>(Event::default().data(chunk.to_string()))),
    )
    .chain(stream::once(async {
        Ok::<_, Infallible>(Event::default().data("[DONE]"))
    }));

    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

async fn anthropic_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    if let Some(response) = unauthorized_if_needed(&state, &headers) {
        return response;
    }

    let Some(model_alias) = request.get("model").and_then(Value::as_str) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request is missing string field `model`",
        );
    };
    let model_alias = model_alias.to_string();

    match handle_anthropic_alias(&state, &model_alias, request).await {
        Ok(response) => response,
        Err(error) => normalized_error_response(error),
    }
}

async fn handle_anthropic_alias(
    state: &AppState,
    alias: &str,
    request: Value,
) -> Result<Response, NormalizedError> {
    let route = state
        .config
        .models
        .get(alias)
        .ok_or_else(|| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("unknown model alias `{alias}`"),
            raw_debug: None,
        })?;

    let provider = state
        .config
        .providers
        .get(&route.provider)
        .ok_or_else(|| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("model alias `{alias}` references unknown provider"),
            raw_debug: None,
        })?;

    if provider.provider_type == ProviderType::AnthropicCompatible {
        proxy_anthropic_compatible(state, provider, &route.model, request).await
    } else {
        Ok(mock_anthropic_response(alias.to_string(), request))
    }
}

async fn proxy_anthropic_compatible(
    state: &AppState,
    provider: &ProviderConfig,
    upstream_model: &str,
    mut request: Value,
) -> Result<Response, NormalizedError> {
    let Some(base_url) = provider.base_url.as_deref() else {
        return Err(NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: "anthropic-compatible provider is missing base_url".to_string(),
            raw_debug: None,
        });
    };

    let api_key = match provider.api_key_env.as_deref() {
        Some(env_name) => Some(std::env::var(env_name).map_err(|error| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!(
                "anthropic-compatible provider API key env `{env_name}` is not set"
            ),
            raw_debug: Some(error.to_string()),
        })?),
        None => None,
    };

    let stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    request["model"] = Value::String(upstream_model.to_string());
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let mut upstream_request = state
        .http
        .post(url)
        .header("anthropic-version", "2023-06-01")
        .json(&request);
    if let Some(api_key) = api_key {
        upstream_request = upstream_request.header("x-api-key", api_key);
    }

    let upstream = upstream_request
        .send()
        .await
        .map_err(classify_reqwest_error)?;
    let status = upstream.status();

    if stream {
        let content_type = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .cloned()
            .unwrap_or_else(|| header::HeaderValue::from_static("text/event-stream"));
        let body = Body::from_stream(
            upstream
                .bytes_stream()
                .map(|item| item.map_err(std::io::Error::other)),
        );

        Ok(Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .unwrap_or_else(|error| {
                tracing::error!(error = %error, "failed to build anthropic streaming response");
                normalized_error_response(NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 500,
                    provider_code: None,
                    safe_message: "failed to build anthropic streaming response".to_string(),
                    raw_debug: Some(error.to_string()),
                })
            }))
    } else {
        let body = upstream.bytes().await.map_err(|error| NormalizedError {
            category: ErrorCategory::Network,
            retryable: true,
            http_status: 502,
            provider_code: None,
            safe_message: "failed to read upstream response body".to_string(),
            raw_debug: Some(error.to_string()),
        })?;

        Ok(Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap_or_else(|error| {
                tracing::error!(error = %error, "failed to build anthropic upstream response");
                normalized_error_response(NormalizedError {
                    category: ErrorCategory::ProviderBug,
                    retryable: false,
                    http_status: 500,
                    provider_code: None,
                    safe_message: "failed to build anthropic upstream response".to_string(),
                    raw_debug: Some(error.to_string()),
                })
            }))
    }
}

fn mock_anthropic_response(model: String, request: Value) -> Response {
    if request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        mock_anthropic_stream(model)
    } else {
        mock_anthropic_json(model)
    }
}

fn mock_anthropic_json(model: String) -> Response {
    Json(json!({
        "id": "msg_ma_compat",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{
            "type": "text",
            "text": "Models Assemble compat-probe OK."
        }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 0,
            "output_tokens": 0
        }
    }))
    .into_response()
}

fn mock_anthropic_stream(model: String) -> Response {
    let events = vec![
        Event::default().event("message_start").data(
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_ma_compat",
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                }
            })
            .to_string(),
        ),
        Event::default().event("content_block_start").data(
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            })
            .to_string(),
        ),
        Event::default().event("content_block_delta").data(
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "Models Assemble compat-probe OK." }
            })
            .to_string(),
        ),
        Event::default()
            .event("content_block_stop")
            .data(json!({ "type": "content_block_stop", "index": 0 }).to_string()),
        Event::default().event("message_delta").data(
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": { "output_tokens": 0 }
            })
            .to_string(),
        ),
        Event::default()
            .event("message_stop")
            .data(json!({ "type": "message_stop" }).to_string()),
    ];

    Sse::new(stream::iter(events.into_iter().map(Ok::<_, Infallible>)))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn unauthorized_if_needed(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<axum::response::Response> {
    if state.config.server.api_keys.is_empty() {
        return None;
    }

    let token = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let authorized = token
        .map(|token| state.config.server.api_keys.iter().any(|key| key == token))
        .unwrap_or(false);

    if authorized {
        None
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "type": "auth",
                        "message": "missing or invalid local API key"
                    }
                })),
            )
                .into_response(),
        )
    }
}

fn error_response(
    status: StatusCode,
    error_type: &'static str,
    message: impl Into<String>,
) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "type": error_type,
                "message": message.into()
            }
        })),
    )
        .into_response()
}

fn normalized_error_response(error: NormalizedError) -> Response {
    let status = StatusCode::from_u16(error.http_status).unwrap_or(StatusCode::BAD_GATEWAY);
    error_response(status, error.category.as_str(), error.safe_message)
}

fn classify_reqwest_error(error: reqwest::Error) -> NormalizedError {
    let category = if error.is_timeout() {
        ErrorCategory::Timeout
    } else if error.is_connect() {
        ErrorCategory::Network
    } else {
        ErrorCategory::Network
    };

    NormalizedError {
        category,
        retryable: true,
        http_status: 502,
        provider_code: None,
        safe_message: "upstream provider request failed".to_string(),
        raw_debug: Some(error.to_string()),
    }
}

fn should_fallback_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::BAD_GATEWAY
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status == StatusCode::GATEWAY_TIMEOUT
        || status.is_server_error()
}

fn classify_status(status: StatusCode) -> ErrorCategory {
    match status {
        StatusCode::TOO_MANY_REQUESTS => ErrorCategory::RateLimited,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ErrorCategory::Timeout,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => ErrorCategory::Overloaded,
        status if status.is_server_error() => ErrorCategory::Overloaded,
        _ => ErrorCategory::Unknown,
    }
}

pub fn compat_config(bind: SocketAddr) -> AppConfig {
    AppConfig {
        server: ma_core::ServerConfig {
            bind,
            api_keys: Vec::new(),
        },
        ..AppConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use http::HeaderMap;
    use ma_core::{
        AppConfig, ProviderConfig, ProviderType, ServerConfig,
        config::{ModelRoute, ProviderCompliance, RoutingConfig},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn proxies_openai_compatible_non_stream_request() {
        let upstream = Router::new()
            .route("/v1/models", get(|| async { Json(json!({"data": []})) }))
            .route(
                "/v1/chat/completions",
                post(|Json(body): Json<Value>| async move {
                    Json(json!({
                        "id": "upstream-test",
                        "object": "chat.completion",
                        "model": body["model"],
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "proxied"
                            },
                            "finish_reason": "stop"
                        }]
                    }))
                }),
            );
        let upstream_addr = spawn_test_server(upstream).await;

        let app = router(test_config(upstream_addr));
        let response = app
            .oneshot(openai_request("assemble-main", false))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["id"], "upstream-test");
        assert_eq!(body["model"], "real-upstream-model");
        assert_eq!(body["choices"][0]["message"]["content"], "proxied");
    }

    #[tokio::test]
    async fn proxies_openai_compatible_stream_request() {
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(|Json(_body): Json<Value>| async move {
                (
                    [(http::header::CONTENT_TYPE, "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"proxied\"}}]}\n\ndata: [DONE]\n\n",
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let app = router(test_config(upstream_addr));
        let response = app
            .oneshot(openai_request("assemble-main", true))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("data: {\"choices\""));
        assert!(body.contains("data: [DONE]"));
    }

    #[tokio::test]
    async fn falls_back_on_retryable_status_for_non_stream_request() {
        let primary_hits = Arc::new(AtomicUsize::new(0));
        let primary_hits_for_route = Arc::clone(&primary_hits);
        let primary = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(_body): Json<Value>| {
                let primary_hits = Arc::clone(&primary_hits_for_route);
                async move {
                    primary_hits.fetch_add(1, Ordering::SeqCst);
                    (
                        http::StatusCode::TOO_MANY_REQUESTS,
                        Json(json!({"error": {"message": "rate limited"}})),
                    )
                }
            }),
        );
        let fallback = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                Json(json!({
                    "id": "fallback-test",
                    "object": "chat.completion",
                    "model": body["model"],
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "fallback"
                        },
                        "finish_reason": "stop"
                    }]
                }))
            }),
        );
        let primary_addr = spawn_test_server(primary).await;
        let fallback_addr = spawn_test_server(fallback).await;

        let app = router(test_config_with_fallback(primary_addr, fallback_addr));
        let response = app
            .oneshot(openai_request("assemble-main", false))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["id"], "fallback-test");
        assert_eq!(body["model"], "fallback-upstream-model");
        assert_eq!(body["choices"][0]["message"]["content"], "fallback");
        assert_eq!(primary_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn does_not_fallback_on_non_retryable_status() {
        let fallback_hits = Arc::new(AtomicUsize::new(0));
        let fallback_hits_for_route = Arc::clone(&fallback_hits);
        let primary = Router::new().route(
            "/v1/chat/completions",
            post(|Json(_body): Json<Value>| async move {
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"message": "bad request"}})),
                )
            }),
        );
        let fallback = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(_body): Json<Value>| {
                let fallback_hits = Arc::clone(&fallback_hits_for_route);
                async move {
                    fallback_hits.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"id": "should-not-run"}))
                }
            }),
        );
        let primary_addr = spawn_test_server(primary).await;
        let fallback_addr = spawn_test_server(fallback).await;

        let app = router(test_config_with_fallback(primary_addr, fallback_addr));
        let response = app
            .oneshot(openai_request("assemble-main", false))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["message"], "bad request");
        assert_eq!(fallback_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn proxies_anthropic_compatible_non_stream_request() {
        let upstream = Router::new().route(
            "/v1/messages",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(
                    headers
                        .get("anthropic-version")
                        .and_then(|v| v.to_str().ok()),
                    Some("2023-06-01")
                );
                Json(json!({
                    "id": "msg_upstream_test",
                    "type": "message",
                    "role": "assistant",
                    "model": body["model"],
                    "content": [{
                        "type": "text",
                        "text": "anthropic proxied"
                    }],
                    "stop_reason": "end_turn",
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 2
                    }
                }))
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let app = router(test_anthropic_config(upstream_addr));
        let response = app
            .oneshot(anthropic_request("assemble-claude", false))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["id"], "msg_upstream_test");
        assert_eq!(body["model"], "claude-upstream-model");
        assert_eq!(body["content"][0]["text"], "anthropic proxied");
    }

    #[tokio::test]
    async fn proxies_anthropic_compatible_stream_request() {
        let upstream = Router::new().route(
            "/v1/messages",
            post(|Json(_body): Json<Value>| async move {
                (
                    [(http::header::CONTENT_TYPE, "text/event-stream")],
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"proxied\"}}\n\n",
                )
            }),
        );
        let upstream_addr = spawn_test_server(upstream).await;

        let app = router(test_anthropic_config(upstream_addr));
        let response = app
            .oneshot(anthropic_request("assemble-claude", true))
            .await
            .unwrap();

        assert_eq!(response.status(), http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("proxied"));
    }

    fn test_config(upstream_addr: SocketAddr) -> AppConfig {
        let mut models = BTreeMap::new();
        models.insert(
            "assemble-main".to_string(),
            ModelRoute {
                provider: "test-openai".to_string(),
                model: "real-upstream-model".to_string(),
            },
        );

        let mut providers = BTreeMap::new();
        providers.insert(
            "test-openai".to_string(),
            ProviderConfig {
                provider_type: ProviderType::OpenAiCompatible,
                base_url: Some(format!("http://{upstream_addr}/v1")),
                api_key_env: None,
                compliance: ProviderCompliance::OfficialApi,
            },
        );

        AppConfig {
            server: ServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                api_keys: Vec::new(),
            },
            models,
            providers,
            routing: RoutingConfig {
                default: "assemble-main".to_string(),
            },
            fallback: BTreeMap::new(),
        }
    }

    fn test_config_with_fallback(primary_addr: SocketAddr, fallback_addr: SocketAddr) -> AppConfig {
        let mut config = test_config(primary_addr);
        config.models.insert(
            "assemble-cheap".to_string(),
            ModelRoute {
                provider: "fallback-openai".to_string(),
                model: "fallback-upstream-model".to_string(),
            },
        );
        config.providers.insert(
            "fallback-openai".to_string(),
            ProviderConfig {
                provider_type: ProviderType::OpenAiCompatible,
                base_url: Some(format!("http://{fallback_addr}/v1")),
                api_key_env: None,
                compliance: ProviderCompliance::OfficialApi,
            },
        );
        config.fallback.insert(
            "assemble-main".to_string(),
            vec!["assemble-cheap".to_string()],
        );
        config
    }

    fn test_anthropic_config(upstream_addr: SocketAddr) -> AppConfig {
        let mut models = BTreeMap::new();
        models.insert(
            "assemble-claude".to_string(),
            ModelRoute {
                provider: "test-anthropic".to_string(),
                model: "claude-upstream-model".to_string(),
            },
        );

        let mut providers = BTreeMap::new();
        providers.insert(
            "test-anthropic".to_string(),
            ProviderConfig {
                provider_type: ProviderType::AnthropicCompatible,
                base_url: Some(format!("http://{upstream_addr}/v1")),
                api_key_env: None,
                compliance: ProviderCompliance::OfficialApi,
            },
        );

        AppConfig {
            server: ServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                api_keys: Vec::new(),
            },
            models,
            providers,
            routing: RoutingConfig {
                default: "assemble-claude".to_string(),
            },
            fallback: BTreeMap::new(),
        }
    }

    fn openai_request(model: &str, stream: bool) -> http::Request<axum::body::Body> {
        http::Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({
                    "model": model,
                    "messages": [{ "role": "user", "content": "ping" }],
                    "stream": stream
                })
                .to_string(),
            ))
            .unwrap()
    }

    fn anthropic_request(model: &str, stream: bool) -> http::Request<axum::body::Body> {
        http::Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({
                    "model": model,
                    "messages": [{ "role": "user", "content": "ping" }],
                    "max_tokens": 64,
                    "stream": stream
                })
                .to_string(),
            ))
            .unwrap()
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn spawn_test_server(app: Router) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }
}
