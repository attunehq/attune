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
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListRepositoryRequest {}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListRepositoryResponse {
    pub repositories: Vec<Repository>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
) -> Result<Json<ListRepositoryResponse>, ErrorResponse> {
    let repositories = sqlx::query!(
        r#"
        SELECT id, name
        FROM debian_repository
        WHERE tenant_id = $1
        ORDER BY created_at ASC
        "#,
        tenant_id.0,
    )
    .fetch_all(&state.db)
    .await
    .map_err(ErrorResponse::from)?;
    let repositories = repositories
        .into_iter()
        .map(|r| Repository {
            id: r.id,
            name: r.name,
        })
        .collect();
    Ok(Json(ListRepositoryResponse { repositories }))
}
