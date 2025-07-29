use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::decode_repo_name},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Repository {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EditRepositoryRequest {
    pub new_name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct EditRepositoryResponse {
    pub result: Repository,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(name): Path<String>,
    Json(req): Json<EditRepositoryRequest>,
) -> Result<Json<EditRepositoryResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let name = decode_repo_name(&name)?;

    let updated = sqlx::query!(
        r#"
        UPDATE debian_repository
        SET name = $3
        WHERE tenant_id = $1 AND name = $2
        RETURNING id, name
        "#,
        tenant_id.0,
        &name,
        req.new_name.unwrap_or(name.to_string()),
    )
    .fetch_optional(&state.db)
    .await
    .unwrap();
    match updated {
        Some(updated) => Ok(Json(EditRepositoryResponse {
            result: Repository { name: updated.name },
        })),
        None => Err(ErrorResponse::new(
            StatusCode::NOT_FOUND,
            "REPO_NOT_FOUND".to_string(),
            "repository not found".to_string(),
        )),
    }
}
