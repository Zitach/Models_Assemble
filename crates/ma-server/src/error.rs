use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use ma_core::{ErrorCategory, NormalizedError};
use serde_json::json;

pub fn error_response(
    status: StatusCode,
    error_type: impl Into<String>,
    message: impl Into<String>,
) -> Response {
    let error_type = error_type.into();
    let message = message.into();
    (
        status,
        Json(json!({
            "error": {
                "type": error_type,
                "message": message
            }
        })),
    )
        .into_response()
}

pub fn normalized_error_response(error: NormalizedError) -> Response {
    let status = StatusCode::from_u16(error.http_status).unwrap_or(StatusCode::BAD_GATEWAY);
    error_response(status, error.category.as_str(), error.safe_message)
}

pub fn classify_reqwest_error(error: reqwest::Error) -> NormalizedError {
    let category = if error.is_timeout() {
        ErrorCategory::Timeout
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

pub fn should_fallback_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::BAD_GATEWAY
        || status == StatusCode::SERVICE_UNAVAILABLE
        || status == StatusCode::GATEWAY_TIMEOUT
        || status.is_server_error()
}

pub fn classify_status(status: StatusCode) -> ErrorCategory {
    match status {
        StatusCode::TOO_MANY_REQUESTS => ErrorCategory::RateLimited,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ErrorCategory::Timeout,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE => ErrorCategory::Overloaded,
        status if status.is_server_error() => ErrorCategory::Overloaded,
        _ => ErrorCategory::Unknown,
    }
}
