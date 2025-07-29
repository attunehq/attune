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
pub struct RepositoryInfoResponse {
    pub name: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
) -> Result<Json<RepositoryInfoResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repository_name = decode_repo_name(&repository_name)?;

    let repo = sqlx::query!(
        r#"
        SELECT name
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        LIMIT 1
        "#,
        tenant_id.0,
        repository_name,
    )
    .fetch_optional(&state.db)
    .await
    .unwrap();
    match repo {
        Some(repo) => Ok(Json(RepositoryInfoResponse { name: repo.name })),
        None => Err(ErrorResponse::new(
            StatusCode::NOT_FOUND,
            "REPO_NOT_FOUND".to_string(),
            "repository not found".to_string(),
        )),
    }
}
