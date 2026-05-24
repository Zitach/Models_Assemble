use std::{collections::BTreeMap, fs, io::ErrorKind, net::SocketAddr};

use ma_core::{
    AppConfig, ProviderCompliance, ProviderConfig, ProviderTestResult, ProviderType, ServerConfig,
    config::{ModelRoute, RoutingConfig},
    provider_test,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Serialize)]
struct GatewayStatus {
    running: bool,
    bind: &'static str,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiPlan {
    id: String,
    name: String,
    provider_id: String,
    protocol: String,
    base_url: String,
    api_key_env: String,
    api_key_preview: String,
    models: Vec<String>,
    main_model: String,
    fast_model: String,
    max_model: String,
    subagent_model: String,
    request_overrides: String,
    status: String,
    template: String,
    last_test: String,
}

#[tauri::command]
fn get_gateway_status() -> GatewayStatus {
    GatewayStatus {
        running: false,
        bind: "127.0.0.1:8787",
    }
}

#[tauri::command]
async fn test_provider(plan: UiPlan, stream: bool) -> Result<ProviderTestResult, String> {
    let provider = plan.to_provider_config()?;
    let route = plan.to_model_route()?;
    let api_key_override = plan.api_key_preview.trim();
    provider_test::test_provider_route(
        &provider,
        &route,
        stream,
        (!api_key_override.is_empty()).then_some(api_key_override),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_plans(app: tauri::AppHandle, plans: Vec<UiPlan>) -> Result<ProviderTestResult, String> {
    let count = plans.len();
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|error| error.to_string())?;

    let sanitized_plans = plans.iter().map(UiPlan::without_secret).collect::<Vec<_>>();
    let plans_path = config_dir.join("plans.json");
    let plans_json =
        serde_json::to_string_pretty(&sanitized_plans).map_err(|error| error.to_string())?;
    fs::write(&plans_path, plans_json).map_err(|error| error.to_string())?;

    let gateway_config = build_app_config(&plans)?;
    let config_yaml = serde_yaml::to_string(&gateway_config).map_err(|error| error.to_string())?;
    let config_path = config_dir.join("config.yaml");
    fs::write(&config_path, config_yaml).map_err(|error| error.to_string())?;

    Ok(ProviderTestResult {
        ok: true,
        status: "saved".to_string(),
        text_preview: format!("{count} plan(s) saved to {}", config_dir.display()),
    })
}

#[tauri::command]
fn load_plans(app: tauri::AppHandle) -> Result<Vec<UiPlan>, String> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let plans_path = config_dir.join("plans.json");

    match fs::read_to_string(&plans_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

impl UiPlan {
    fn to_provider_config(&self) -> Result<ProviderConfig, String> {
        Ok(ProviderConfig {
            provider_type: parse_provider_type(&self.protocol)?,
            base_url: Some(self.base_url.trim().to_string()).filter(|value| !value.is_empty()),
            api_key_env: Some(self.api_key_env.trim().to_string())
                .filter(|value| !value.is_empty()),
            compliance: ProviderCompliance::OfficialCodingEndpoint,
        })
    }

    fn to_model_route(&self) -> Result<ModelRoute, String> {
        let model = fallback_model(&self.main_model, &self.models)?;

        Ok(ModelRoute {
            provider: self.provider_id.clone(),
            model,
        })
    }

    fn without_secret(&self) -> Self {
        Self {
            api_key_preview: String::new(),
            ..self.clone()
        }
    }
}

fn build_app_config(plans: &[UiPlan]) -> Result<AppConfig, String> {
    let mut models = BTreeMap::new();
    let mut providers = BTreeMap::new();

    for plan in plans {
        models.insert(plan.id.clone(), plan.to_model_route()?);
        models.insert(
            format!("{}-fast", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.fast_model, &plan.models)?,
            },
        );
        models.insert(
            format!("{}-max", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.max_model, &plan.models)?,
            },
        );
        models.insert(
            format!("{}-subagent", plan.id),
            ModelRoute {
                provider: plan.provider_id.clone(),
                model: fallback_model(&plan.subagent_model, &plan.models)?,
            },
        );
        providers.insert(plan.provider_id.clone(), plan.to_provider_config()?);
    }

    let default = plans
        .first()
        .map(|plan| plan.id.clone())
        .unwrap_or_else(|| "assemble-mock".to_string());

    Ok(AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:8787"
                .parse::<SocketAddr>()
                .expect("default desktop bind address is valid"),
            api_keys: vec!["ma-local-dev-key".to_string()],
        },
        models,
        providers,
        routing: RoutingConfig { default },
        fallback: BTreeMap::new(),
    })
}

fn fallback_model(value: &str, models: &[String]) -> Result<String, String> {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    models
        .first()
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "plan must include at least one model".to_string())
}

fn parse_provider_type(protocol: &str) -> Result<ProviderType, String> {
    match protocol {
        "anthropic_compatible" => Ok(ProviderType::AnthropicCompatible),
        "openai_compatible" => Ok(ProviderType::OpenAiCompatible),
        other => Err(format!("unsupported provider protocol `{other}`")),
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            test_provider,
            save_plans,
            load_plans
        ])
        .run(tauri::generate_context!())
        .expect("error while running Models Assemble desktop");
}
