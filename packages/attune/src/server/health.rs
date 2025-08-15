use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::{api::ErrorResponse, server::ServerState};

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthCheckResponse {
    ready: bool,
}

#[axum::debug_handler]
pub async fn handler(
    State(state): State<ServerState>,
) -> Result<Json<HealthCheckResponse>, ErrorResponse> {
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(Json(HealthCheckResponse { ready: true }))
}
