use std::{collections::BTreeMap, fs, net::SocketAddr, path::Path};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCategory, NormalizedError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub models: BTreeMap<String, ModelRoute>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub fallback: BTreeMap<String, Vec<String>>,
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, NormalizedError> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).map_err(|err| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("failed to read config file {}", path.display()),
            raw_debug: Some(err.to_string()),
        })?;

        serde_yaml::from_str(&raw).map_err(|err| NormalizedError {
            category: ErrorCategory::InvalidRequest,
            retryable: false,
            http_status: 400,
            provider_code: None,
            safe_message: format!("failed to parse config file {}", path.display()),
            raw_debug: Some(err.to_string()),
        })
    }

    pub fn validate(&self) -> Result<(), Vec<NormalizedError>> {
        let mut errors = Vec::new();

        for (alias, route) in &self.models {
            if !self.providers.contains_key(&route.provider) {
                errors.push(NormalizedError {
                    category: ErrorCategory::InvalidRequest,
                    retryable: false,
                    http_status: 400,
                    provider_code: None,
                    safe_message: format!(
                        "model alias `{alias}` references unknown provider `{}`",
                        route.provider
                    ),
                    raw_debug: None,
                });
            }
        }

        if !self.models.contains_key(&self.routing.default) && !self.routing.default.is_empty() {
            errors.push(NormalizedError {
                category: ErrorCategory::InvalidRequest,
                retryable: false,
                http_status: 400,
                provider_code: None,
                safe_message: format!(
                    "routing.default references unknown model alias `{}`",
                    self.routing.default
                ),
                raw_debug: None,
            });
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut models = BTreeMap::new();
        models.insert(
            "assemble-mock".to_string(),
            ModelRoute {
                provider: "mock".to_string(),
                model: "mock-coding-model".to_string(),
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

        Self {
            server: ServerConfig::default(),
            models,
            providers,
            routing: RoutingConfig {
                default: "assemble-mock".to_string(),
            },
            fallback: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            api_keys: Vec::new(),
        }
    }
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:8787"
        .parse()
        .expect("default bind address is valid")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub compliance: ProviderCompliance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Mock,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "anthropic_compatible")]
    AnthropicCompatible,
    ZhipuCodingPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCompliance {
    OfficialApi,
    OfficialCodingEndpoint,
    CompatibleProxy,
    Unsupported,
}

impl Default for ProviderCompliance {
    fn default() -> Self {
        Self::CompatibleProxy
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    #[serde(default = "default_model_alias")]
    pub default: String,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            default: default_model_alias(),
        }
    }
}

fn default_model_alias() -> String {
    "assemble-mock".to_string()
}
