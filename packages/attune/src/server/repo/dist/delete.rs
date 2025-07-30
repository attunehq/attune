use axum::{
    Json,
    extract::{Path, State},
};
use bon::Builder;
use serde::{Deserialize, Serialize};
use tap::Pipe;
use tracing::instrument;

use crate::{api::ErrorResponse, auth::TenantID, server::ServerState};

use super::{decode_dist_name, super::decode_repo_name};

#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct DeleteDistributionResponse {
    #[builder(into)]
    pub message: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repository_name, distribution_name)): Path<(String, String)>,
) -> Result<Json<DeleteDistributionResponse>, ErrorResponse> {
    let repository_name = decode_repo_name(&repository_name)?;
    let distribution_name = decode_dist_name(&distribution_name)?;

    let mut tx = state.db.begin().await.unwrap();
    let repo = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        repository_name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .ok_or_else(|| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("REPO_NOT_FOUND")
            .message("repository not found")
            .build()
    })?;

    // Delete the distribution (cascade will handle related records)
    let result = sqlx::query!(
        r#"
        DELETE FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
        "#,
        repo.id,
        distribution_name,
    )
    .execute(&mut *tx)
    .await
    .unwrap();

    if result.rows_affected() == 0 {
        return ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("DIST_NOT_FOUND")
            .message("distribution not found")
            .build()
            .pipe(Err);
    }

    tx.commit().await.unwrap();

    DeleteDistributionResponse::builder()
        .message(format!("Distribution '{}' deleted successfully", distribution_name))
        .build()
        .pipe(Json)
        .pipe(Ok)
}