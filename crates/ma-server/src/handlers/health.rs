use axum::{Json, extract::State};
use ma_core::{ModelInfo, ModelList, protocol::HealthResponse};

use crate::AppState;

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "models-assemble",
    })
}

pub async fn list_models(State(state): State<AppState>) -> Json<ModelList> {
    let data = state
        .config
        .models
        .keys()
        .map(|id| ModelInfo {
            id: id.clone(),
            object: "model",
            owned_by: "models-assemble",
        })
        .collect();

    Json(ModelList {
        object: "list",
        data,
    })
}
