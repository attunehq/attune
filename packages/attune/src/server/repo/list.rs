use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::ServerState,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub s3_bucket: String,
    pub s3_prefix: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListRepositoryRequest {
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListRepositoryResponse {
    pub repositories: Vec<Repository>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Json(req): Json<ListRepositoryRequest>,
) -> Result<Json<ListRepositoryResponse>, ErrorResponse> {
    // TODO: In the managed cloud version of this CLI, we should hide the S3
    // bucket and prefix fields because they're irrelevant.
    let repositories = sqlx::query!(
        r#"
        SELECT id, name, s3_bucket, s3_prefix
        FROM debian_repository
        WHERE
            tenant_id = $1
            AND name LIKE '%' || $2 || '%'
        ORDER BY created_at ASC
        "#,
        tenant_id.0,
        req.name.unwrap_or_default(),
    )
    .fetch_all(&state.db)
    .await
    .map_err(ErrorResponse::from)?;
    let repositories = repositories
        .into_iter()
        .map(|r| Repository {
            id: r.id,
            name: r.name,
            s3_bucket: r.s3_bucket,
            s3_prefix: r.s3_prefix,
        })
        .collect();
    Ok(Json(ListRepositoryResponse { repositories }))
}
