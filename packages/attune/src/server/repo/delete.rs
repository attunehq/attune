use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::{ServerState, repo::decode_repo_name},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct DeleteRepositoryRequest {}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeleteRepositoryResponse {}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(name): Path<String>,
    Json(req): Json<DeleteRepositoryRequest>,
) -> Result<Json<DeleteRepositoryResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let name = decode_repo_name(&name)?;

    let deleted = sqlx::query!(
        r#"
        DELETE FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        &name,
    )
    .execute(&state.db)
    .await
    .map_err(ErrorResponse::from)?;
    if deleted.rows_affected() > 0 {
        Ok(Json(DeleteRepositoryResponse {}))
    } else {
        Err(ErrorResponse::new(
            StatusCode::NOT_FOUND,
            "REPO_NOT_FOUND".to_string(),
            "repository not found".to_string(),
        ))
    }
}
