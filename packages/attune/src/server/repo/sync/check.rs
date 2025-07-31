use axum::{
    Json,
    extract::{Path, State},
};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            sync::{InconsistentObjects, check_consistency},
        },
    },
};

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repo_name, release_name)): Path<(String, String)>,
) -> Result<Json<InconsistentObjects>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;
    let release_name = decode_repo_name(&release_name)?;

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    let inconsistent_objects =
        check_consistency(&mut tx, state.s3, &tenant_id, repo_name, release_name).await?;

    tx.commit().await.unwrap();

    Ok(Json(inconsistent_objects))
}
