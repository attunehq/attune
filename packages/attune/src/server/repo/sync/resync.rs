use axum::{
    Json,
    extract::{Path, State},
};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::decode_repo_name},
};

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repo_name): Path<String>,
) -> Result<Json<()>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    #[allow(unused_variables)]
    let repo_name = decode_repo_name(&repo_name)?;

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    todo!();

    #[allow(unreachable_code)]
    tx.commit().await.unwrap();

    todo!()
}
