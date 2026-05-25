use std::net::SocketAddr;

use axum::Router;
use serde_json::{Value, json};
use tokio::net::TcpListener;

pub mod normalized_test;

pub fn openai_request(model: &str, stream: bool) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({
                "model": model,
                "messages": [{ "role": "user", "content": "ping" }],
                "stream": stream
            })
            .to_string(),
        ))
        .unwrap()
}

pub fn anthropic_request(model: &str, stream: bool) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({
                "model": model,
                "messages": [{ "role": "user", "content": "ping" }],
                "max_tokens": 64,
                "stream": stream
            })
            .to_string(),
        ))
        .unwrap()
}

pub async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

pub async fn spawn_test_server(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}
