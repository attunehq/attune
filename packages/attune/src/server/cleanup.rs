use axum::extract::State;
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::sync::cleanup_expired_by_hash_objects},
};

/// HTTP handler to clean up expired historical by-hash index files.
/// This endpoint should be called periodically (e.g., via cron job) to maintain
/// the retention policy for by-hash historical versions.
///
/// Note: This only cleans up historical versions that were scheduled for cleanup
/// when indexes were updated. Immediate deletions (package/distribution removal)
/// do not use this cleanup system.
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