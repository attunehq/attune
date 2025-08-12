use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::{
    api::{ErrorResponse, TenantID},
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            sync::{InconsistentSummary, check_s3_consistency, query_repository_state},
        },
    },
};

#[derive(Debug, Serialize, Deserialize)]
pub struct CheckConsistencyResponse {
    #[serde(flatten)]
    pub status: InconsistentSummary,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repo_name, release_name)): Path<(String, String)>,
) -> Result<Json<CheckConsistencyResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;
    let release_name = decode_repo_name(&release_name)?;

    // Get current repository state.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(ErrorResponse::from)?;
    let repo = query_repository_state(&mut tx, &tenant_id, repo_name, release_name).await?;
    tx.commit().await.map_err(ErrorResponse::from)?;
    debug!(?repo, "loaded repository state");

    // Check which S3 objects are inconsistent.
    let inconsistent_objects = check_s3_consistency(&state.s3, repo).await?;
    debug!(?inconsistent_objects, "checked S3");

    Ok(Json(CheckConsistencyResponse {
        status: InconsistentSummary::from(&inconsistent_objects),
    }))
}
