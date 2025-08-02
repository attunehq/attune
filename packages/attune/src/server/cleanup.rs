use axum::extract::State;
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::sync::cleanup_expired_by_hash_objects},
};

/// HTTP handler to clean up expired historical index files.
/// This endpoint can be called periodically (e.g., via cron job) to maintain
/// the retention policy for by-hash historical versions.
#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn cleanup_handler(
    State(state): State<ServerState>,
    _tenant_id: TenantID,
) -> Result<(), ErrorResponse> {
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    cleanup_expired_by_hash_objects(&mut tx, state.s3.clone()).await?;

    tx.commit().await.unwrap();

    Ok(())
}
