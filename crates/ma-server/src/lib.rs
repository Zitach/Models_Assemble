use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    Router,
    middleware::from_fn,
    routing::{get, post},
};
use ma_core::adapter::ProviderAdapter;
use ma_core::{AppConfig, ProviderType};
use reqwest::Client;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

pub mod adapters;
pub mod auth;
pub mod error;
pub mod fallback;
pub mod handlers;
pub mod middleware;
#[cfg(test)]
pub mod tests;

use middleware::request_id::request_id_middleware;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub http: Client,
    pub adapter_registry: Arc<HashMap<String, Arc<dyn ProviderAdapter>>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let adapter_registry = build_adapter_registry(&config);
        Self {
            config: Arc::new(config),
            http: Client::new(),
            adapter_registry: Arc::new(adapter_registry),
        }
    }
}

fn build_adapter_registry(config: &AppConfig) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut registry = HashMap::new();

    for (provider_name, provider_config) in &config.providers {
        let adapter: Arc<dyn ProviderAdapter> = match provider_config.provider_type {
            ProviderType::OpenAiCompatible => {
                let base_url = provider_config.base_url.clone().unwrap_or_default();
                let api_key = provider_config
                    .api_key_env
                    .as_ref()
                    .and_then(|env_name| std::env::var(env_name).ok());
                Arc::new(adapters::openai::OpenAiAdapter::new(
                    provider_name.clone(),
                    base_url,
                    api_key,
                    "",
                ))
            }
            ProviderType::AnthropicCompatible => {
                let base_url = provider_config.base_url.clone().unwrap_or_default();
                let api_key = provider_config
                    .api_key_env
                    .as_ref()
                    .and_then(|env_name| std::env::var(env_name).ok());
                Arc::new(adapters::anthropic::AnthropicAdapter::new(
                    provider_name.clone(),
                    base_url,
                    api_key,
                    "",
                ))
            }
            ProviderType::Mock => Arc::new(adapters::mock::MockAdapter::new()),
            ProviderType::ZhipuCodingPlan => Arc::new(adapters::mock::MockAdapter::new()),
        };

        registry.insert(provider_name.clone(), adapter);
    }

    registry
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
        .route("/health", get(handlers::health::health))
        .route("/v1/models", get(handlers::health::list_models))
        .route(
            "/v1/chat/completions",
            post(handlers::openai::openai_chat_completions),
        )
        .route(
            "/v1/messages",
            post(handlers::anthropic::anthropic_messages),
        )
        .layer(from_fn(request_id_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
