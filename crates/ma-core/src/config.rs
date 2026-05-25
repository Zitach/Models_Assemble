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

        for alias in self.models.keys() {
            if alias.contains(' ') {
                errors.push(NormalizedError {
                    category: ErrorCategory::InvalidRequest,
                    retryable: false,
                    http_status: 400,
                    provider_code: None,
                    safe_message: format!("model alias `{alias}` contains spaces"),
                    raw_debug: None,
                });
            }
        }

        for (provider_name, provider) in &self.providers {
            if let Some(ref url) = provider.base_url
                && !url.starts_with("http://")
                && !url.starts_with("https://")
            {
                errors.push(NormalizedError {
                    category: ErrorCategory::InvalidRequest,
                    retryable: false,
                    http_status: 400,
                    provider_code: None,
                    safe_message: format!(
                        "provider `{provider_name}` base_url must start with http:// or https://"
                    ),
                    raw_debug: None,
                });
            }

            if let Some(ref env) = provider.api_key_env
                && env.trim().is_empty()
            {
                errors.push(NormalizedError {
                    category: ErrorCategory::InvalidRequest,
                    retryable: false,
                    http_status: 400,
                    provider_code: None,
                    safe_message: format!("provider `{provider_name}` api_key_env cannot be empty"),
                    raw_debug: None,
                });
            }

            if provider.provider_type != ProviderType::Mock && provider.api_key_env.is_none() {
                errors.push(NormalizedError {
                    category: ErrorCategory::InvalidRequest,
                    retryable: false,
                    http_status: 400,
                    provider_code: None,
                    safe_message: format!(
                        "provider `{provider_name}` is missing api_key_env (required for non-mock providers)"
                    ),
                    raw_debug: None,
                });
            }
        }

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

        self.detect_circular_fallbacks(&mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn detect_circular_fallbacks(&self, errors: &mut Vec<NormalizedError>) {
        for start_alias in self.fallback.keys() {
            let mut visited = std::collections::HashSet::new();
            let mut stack = vec![start_alias.clone()];

            while let Some(current) = stack.pop() {
                if !visited.insert(current.clone()) {
                    errors.push(NormalizedError {
                        category: ErrorCategory::InvalidRequest,
                        retryable: false,
                        http_status: 400,
                        provider_code: None,
                        safe_message: format!(
                            "circular fallback chain detected starting from `{start_alias}`"
                        ),
                        raw_debug: None,
                    });
                    break;
                }

                if let Some(next_aliases) = self.fallback.get(&current) {
                    for next in next_aliases {
                        stack.push(next.clone());
                    }
                }
            }
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
    #[serde(default = "default_first_token_timeout_secs")]
    pub first_token_timeout_secs: Option<u64>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            api_keys: Vec::new(),
            first_token_timeout_secs: default_first_token_timeout_secs(),
        }
    }
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:8787"
        .parse()
        .expect("default bind address is valid")
}

fn default_first_token_timeout_secs() -> Option<u64> {
    Some(15)
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCompliance {
    OfficialApi,
    OfficialCodingEndpoint,
    #[default]
    CompatibleProxy,
    Unsupported,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn valid_config_passes() {
        let config = make_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn model_alias_with_spaces_fails() {
        let mut config = make_config();
        config.models.insert(
            "bad alias".to_string(),
            ModelRoute {
                provider: "mock".to_string(),
                model: "test".to_string(),
            },
        );
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.safe_message.contains("contains spaces"))
        );
    }

    #[test]
    fn base_url_must_start_with_http() {
        let mut config = make_config();
        config.providers.insert(
            "bad-url".to_string(),
            ProviderConfig {
                provider_type: ProviderType::OpenAiCompatible,
                base_url: Some("ftp://example.com".to_string()),
                api_key_env: Some("KEY".to_string()),
                compliance: ProviderCompliance::OfficialApi,
            },
        );
        config.models.insert(
            "test-model".to_string(),
            ModelRoute {
                provider: "bad-url".to_string(),
                model: "test".to_string(),
            },
        );
        let errs = config.validate().unwrap_err();
        assert!(errs.iter().any(|e| {
            e.safe_message
                .contains("must start with http:// or https://")
        }));
    }

    #[test]
    fn empty_api_key_env_fails() {
        let mut config = make_config();
        config.providers.insert(
            "bad-key".to_string(),
            ProviderConfig {
                provider_type: ProviderType::OpenAiCompatible,
                base_url: Some("https://example.com".to_string()),
                api_key_env: Some("   ".to_string()),
                compliance: ProviderCompliance::OfficialApi,
            },
        );
        config.models.insert(
            "test-model".to_string(),
            ModelRoute {
                provider: "bad-key".to_string(),
                model: "test".to_string(),
            },
        );
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.safe_message.contains("cannot be empty"))
        );
    }

    #[test]
    fn missing_api_key_env_for_non_mock_fails() {
        let mut config = make_config();
        config.providers.insert(
            "no-key".to_string(),
            ProviderConfig {
                provider_type: ProviderType::OpenAiCompatible,
                base_url: Some("https://example.com".to_string()),
                api_key_env: None,
                compliance: ProviderCompliance::OfficialApi,
            },
        );
        config.models.insert(
            "test-model".to_string(),
            ModelRoute {
                provider: "no-key".to_string(),
                model: "test".to_string(),
            },
        );
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.safe_message.contains("missing api_key_env"))
        );
    }

    #[test]
    fn mock_provider_without_api_key_is_ok() {
        let mut config = make_config();
        config.providers.insert(
            "mock2".to_string(),
            ProviderConfig {
                provider_type: ProviderType::Mock,
                base_url: None,
                api_key_env: None,
                compliance: ProviderCompliance::OfficialApi,
            },
        );
        config.models.insert(
            "mock-model".to_string(),
            ModelRoute {
                provider: "mock2".to_string(),
                model: "test".to_string(),
            },
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn circular_fallback_detected() {
        let mut config = make_config();
        config
            .fallback
            .insert("a".to_string(), vec!["b".to_string()]);
        config
            .fallback
            .insert("b".to_string(), vec!["a".to_string()]);
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.safe_message.contains("circular fallback"))
        );
    }

    #[test]
    fn self_referential_fallback_detected() {
        let mut config = make_config();
        config
            .fallback
            .insert("a".to_string(), vec!["a".to_string()]);
        let errs = config.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.safe_message.contains("circular fallback"))
        );
    }

    #[test]
    fn server_config_default_timeout_is_15() {
        let config = ServerConfig::default();
        assert_eq!(config.first_token_timeout_secs, Some(15));
    }
}
