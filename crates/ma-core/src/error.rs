use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedError {
    pub category: ErrorCategory,
    pub retryable: bool,
    pub http_status: u16,
    pub provider_code: Option<String>,
    pub safe_message: String,
    pub raw_debug: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Auth,
    RateLimited,
    Timeout,
    Overloaded,
    InvalidRequest,
    ProviderBug,
    Network,
    Unknown,
}

impl ErrorCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Overloaded => "overloaded",
            Self::InvalidRequest => "invalid_request",
            Self::ProviderBug => "provider_bug",
            Self::Network => "network",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for NormalizedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.safe_message)
    }
}

impl std::error::Error for NormalizedError {}
