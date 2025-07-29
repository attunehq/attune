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
    server::{ServerState, repo::decode_repo_name, repo::index::generate::GenerateIndexRequest},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexRequest {
    pub diff: GenerateIndexRequest,
    pub clearsigned: String,
    pub detachsigned: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum IndexChange {
    Add,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexResponse {}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
    Json(req): Json<SignIndexRequest>,
) -> Result<Json<SignIndexResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repository_name = decode_repo_name(&repository_name)?;

    todo!()
}
