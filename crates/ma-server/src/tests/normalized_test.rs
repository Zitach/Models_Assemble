use std::collections::BTreeMap;

use ma_core::{
    AppConfig, ProviderConfig, ProviderType, ServerConfig,
    config::{ModelRoute, ProviderCompliance, RoutingConfig},
};
use serde_json::json;
use tower::ServiceExt;

use crate::{
    router,
    tests::{anthropic_request, openai_request, response_json},
};

fn test_normalized_config() -> AppConfig {
    let mut models = BTreeMap::new();
    models.insert(
        "assemble-mock".to_string(),
        ModelRoute {
            provider: "mock".to_string(),
            model: "mock-model".to_string(),
        },
    );

    let mut providers = BTreeMap::new();
    providers.insert(
        "mock".to_string(),
        ProviderConfig {
            provider_type: ProviderType::Mock,
            base_url: None,
            api_key_env: None,
            compliance: ProviderCompliance::OfficialApi,
        },
    );

    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            api_keys: Vec::new(),
            first_token_timeout_secs: None,
        },
        models,
        providers,
        routing: RoutingConfig {
            default: "assemble-mock".to_string(),
        },
        fallback: BTreeMap::new(),
    }
}

#[tokio::test]
async fn openai_handler_with_normalized_path_returns_mock_response() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(openai_request("assemble-mock", false))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "Models Assemble compat-probe OK."
    );
}

#[tokio::test]
async fn anthropic_handler_with_normalized_path_returns_mock_response() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(anthropic_request("assemble-mock", false))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(
        body["content"][0]["text"],
        "Models Assemble compat-probe OK."
    );
    assert_eq!(body["stop_reason"], "end_turn");
}

#[tokio::test]
async fn openai_handler_with_normalized_path_stream_returns_mock_events() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(openai_request("assemble-mock", true))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("data: "));
    assert!(body.contains("[DONE]"));
}

#[tokio::test]
async fn anthropic_handler_with_normalized_path_stream_returns_mock_events() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(anthropic_request("assemble-mock", true))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("event: "));
}

#[tokio::test]
async fn openai_handler_unknown_model_returns_400() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(openai_request("unknown-model", false))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert_eq!(
        body["error"]["message"],
        "unknown model alias `unknown-model`"
    );
}

#[tokio::test]
async fn anthropic_handler_unknown_model_returns_400() {
    let app = router(test_normalized_config());
    let response = app
        .oneshot(anthropic_request("unknown-model", false))
        .await
        .unwrap();

    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert_eq!(
        body["error"]["message"],
        "unknown model alias `unknown-model`"
    );
}

#[tokio::test]
async fn anthropic_handler_accepts_x_api_key_on_root_messages_route() {
    let mut config = test_normalized_config();
    config.server.api_keys = vec!["local-token".to_string()];
    let app = router(config);

    let mut request = anthropic_request("assemble-mock", false);
    *request.uri_mut() = "/messages".parse().unwrap();
    request
        .headers_mut()
        .insert("x-api-key", "local-token".parse().unwrap());

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
}

#[tokio::test]
async fn anthropic_compatible_provider_proxies_native_body_without_normalizing() {
    let mut upstream = mockito::Server::new_async().await;
    let mock = upstream
        .mock("POST", "/messages")
        .match_body(mockito::Matcher::PartialJson(json!({
            "model": "upstream-model",
            "tool_choice": { "type": "tool", "name": "search" },
            "unknown_map": { "nested": true }
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "upstream-model",
                "content": [{ "type": "text", "text": "ok" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            })
            .to_string(),
        )
        .create_async()
        .await;

    let mut config = test_normalized_config();
    config.models.insert(
        "assemble-main".to_string(),
        ModelRoute {
            provider: "native".to_string(),
            model: "upstream-model".to_string(),
        },
    );
    config.providers.insert(
        "native".to_string(),
        ProviderConfig {
            provider_type: ProviderType::AnthropicCompatible,
            base_url: Some(upstream.url()),
            api_key_env: None,
            compliance: ProviderCompliance::OfficialCodingEndpoint,
        },
    );

    let app = router(config);
    let request = http::Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({
                "model": "assemble-main",
                "messages": [{ "role": "user", "content": "ping" }],
                "max_tokens": 64,
                "tool_choice": { "type": "tool", "name": "search" },
                "unknown_map": { "nested": true }
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["content"][0]["text"], "ok");
    mock.assert_async().await;
}
