use std::sync::atomic::{AtomicU64, Ordering};

use axum::{extract::Request, middleware::Next, response::Response};
use http::header::HeaderValue;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

pub async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let n = REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            format!("req-{n}")
        });

    tracing::info!(request_id = %request_id, "incoming request");

    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let mut response = next.run(request).await;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }

    response
}

#[cfg(test)]
mod tests {
    use axum::{
        Extension, Router, body::Body, extract::Request, http::StatusCode, middleware,
        response::IntoResponse, routing::get,
    };
    use tower::ServiceExt;

    use super::*;

    async fn echo_request_id(Extension(req_id): Extension<RequestId>) -> impl IntoResponse {
        req_id.0
    }

    fn test_app() -> Router {
        Router::new()
            .route("/test", get(echo_request_id))
            .layer(middleware::from_fn(request_id_middleware))
    }

    #[tokio::test]
    async fn generates_request_id_when_header_missing() {
        let app = test_app();
        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let header = response
            .headers()
            .get("x-request-id")
            .expect("x-request-id header should be present");
        let header_str = header.to_str().unwrap();
        assert!(
            header_str.starts_with("req-"),
            "generated request id should start with req-"
        );
    }

    #[tokio::test]
    async fn preserves_custom_request_id() {
        let app = test_app();
        let custom_id = "my-custom-id-123";
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("x-request-id", custom_id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let header = response
            .headers()
            .get("x-request-id")
            .expect("x-request-id header should be present");
        assert_eq!(header.to_str().unwrap(), custom_id);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(String::from_utf8(body.to_vec()).unwrap(), custom_id);
    }

    #[tokio::test]
    async fn response_includes_request_id_header() {
        let app = test_app();
        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert!(response.headers().contains_key("x-request-id"));
    }
}
