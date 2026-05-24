use anyhow::{Context, bail};
use reqwest::Client;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{AppConfig, ProviderConfig, ProviderType, config::ModelRoute};

#[derive(Debug, Clone, Serialize)]
pub struct ProviderTestResult {
    pub ok: bool,
    pub status: String,
    pub text_preview: String,
}

pub async fn test_provider(
    config: &AppConfig,
    model_alias: &str,
    stream: bool,
) -> anyhow::Result<ProviderTestResult> {
    let route = config
        .models
        .get(model_alias)
        .with_context(|| format!("unknown model alias `{model_alias}`"))?;
    let provider = config
        .providers
        .get(&route.provider)
        .with_context(|| format!("model alias `{model_alias}` references unknown provider"))?;

    test_provider_route(provider, route, stream, None).await
}

pub async fn test_provider_route(
    provider: &ProviderConfig,
    route: &ModelRoute,
    stream: bool,
    api_key_override: Option<&str>,
) -> anyhow::Result<ProviderTestResult> {
    match provider.provider_type {
        ProviderType::Mock => Ok(ProviderTestResult {
            ok: true,
            status: "mock-ok".to_string(),
            text_preview: format!("provider routes to mock model `{}`", route.model),
        }),
        ProviderType::OpenAiCompatible => {
            test_openai_compatible(provider, route, stream, api_key_override).await
        }
        ProviderType::AnthropicCompatible => {
            test_anthropic_compatible(provider, route, stream, api_key_override).await
        }
        ProviderType::ZhipuCodingPlan => {
            bail!("zhipu_coding_plan test-provider is not implemented yet")
        }
    }
}

async fn test_openai_compatible(
    provider: &ProviderConfig,
    route: &ModelRoute,
    stream: bool,
    api_key_override: Option<&str>,
) -> anyhow::Result<ProviderTestResult> {
    let base_url = provider
        .base_url
        .as_deref()
        .context("openai-compatible provider is missing base_url")?;
    let api_key = provider_api_key(provider, api_key_override)?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": route.model,
        "messages": [{ "role": "user", "content": "Reply with exactly: models-assemble-ok" }],
        "stream": stream,
        "max_tokens": 64
    });

    let client = Client::new();
    let mut request = client.post(url).json(&body);
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }

    let response = request.send().await.context("provider request failed")?;
    response_summary(response, stream).await
}

async fn test_anthropic_compatible(
    provider: &ProviderConfig,
    route: &ModelRoute,
    stream: bool,
    api_key_override: Option<&str>,
) -> anyhow::Result<ProviderTestResult> {
    let base_url = provider
        .base_url
        .as_deref()
        .context("anthropic-compatible provider is missing base_url")?;
    let api_key = provider_api_key(provider, api_key_override)?;
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let body = json!({
        "model": route.model,
        "messages": [{ "role": "user", "content": "Reply with exactly: models-assemble-ok" }],
        "stream": stream,
        "max_tokens": 64
    });

    let client = Client::new();
    let mut request = client
        .post(url)
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    if let Some(api_key) = api_key {
        request = request.header("x-api-key", api_key);
    }

    let response = request.send().await.context("provider request failed")?;
    response_summary(response, stream).await
}

fn provider_api_key(
    provider: &ProviderConfig,
    api_key_override: Option<&str>,
) -> anyhow::Result<Option<String>> {
    if let Some(api_key) = api_key_override.filter(|value| !value.is_empty()) {
        return Ok(Some(api_key.to_string()));
    }

    provider
        .api_key_env
        .as_deref()
        .map(|env_name| {
            std::env::var(env_name)
                .with_context(|| format!("required API key env `{env_name}` is not set"))
        })
        .transpose()
}

async fn response_summary(
    response: reqwest::Response,
    stream: bool,
) -> anyhow::Result<ProviderTestResult> {
    let status = response.status();
    let status_text = status.to_string();
    let text = response.text().await.context("failed to read response")?;

    if stream {
        let preview: String = text.chars().take(1000).collect();
        if !status.is_success() {
            bail!("provider stream test failed with status {status}: {preview}");
        }
        return Ok(ProviderTestResult {
            ok: true,
            status: status_text,
            text_preview: preview,
        });
    }

    if !status.is_success() {
        bail!("provider test failed with status {status}: {text}");
    }

    let value: Value = serde_json::from_str(&text).context("provider returned non-JSON body")?;
    Ok(ProviderTestResult {
        ok: true,
        status: status_text,
        text_preview: first_text(&value).unwrap_or_else(|| "<none>".to_string()),
    })
}

pub fn first_text(value: &Value) -> Option<String> {
    if let Some(content) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        return Some(content.chars().take(300).collect());
    }

    value
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|block| {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                block.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .map(|text| text.chars().take(300).collect())
}
