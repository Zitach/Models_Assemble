use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde_json::json;

use crate::AppState;

pub fn unauthorized_if_needed(state: &AppState, headers: &http::HeaderMap) -> Option<Response> {
    if state.config.server.api_keys.is_empty() {
        return None;
    }

    let token = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
        });

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
