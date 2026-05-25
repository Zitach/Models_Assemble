use std::collections::BTreeMap;

use ma_core::{
    AppConfig, ProviderConfig, ProviderType, ServerConfig,
    config::{ModelRoute, ProviderCompliance, RoutingConfig},
};
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
