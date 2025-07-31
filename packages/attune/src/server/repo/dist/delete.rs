use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{
        ServerState,
        repo::{decode_repo_name, dist::decode_dist_name},
    },
};

/// Response after successfully deleting a distribution from a repository.
///
/// Deletion is permanent and will cascade to remove all associated components,
/// package indexes, and package associations. Any packages that were only
/// available through this distribution will become inaccessible until added
/// to another distribution.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct DeleteDistributionResponse {}

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
        SELECT id, s3_bucket, s3_prefix
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
        return Err(ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("DIST_NOT_FOUND")
            .message("distribution not found")
            .build());
    }

    tx.commit().await.unwrap();

    // Clean up S3 objects for this distribution using prefix-based deletion.
    //
    // Note: We don't delete the actual package files (under `/packages/`) because
    // they might be referenced by other distributions in the same or different repositories.
    //
    // As a future improvement we could do garbage collection to delete the actual package files.
    let s3_prefix = format!("{}/dists/{}/", repo.s3_prefix, distribution_name);

    // The continuation token is used to paginate through all the objects.
    let mut continuation_token = None;
    loop {
        let res = state
            .s3
            .list_objects_v2()
            .bucket(&repo.s3_bucket)
            .prefix(&s3_prefix)
            .set_continuation_token(continuation_token)
            .send()
            .await
            .unwrap();

        let targets = res
            .contents()
            .into_iter()
            .filter_map(|obj| obj.key())
            .map(|key| {
                aws_sdk_s3::types::ObjectIdentifier::builder()
                    .key(key)
                    .build()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            let delete = aws_sdk_s3::types::Delete::builder()
                .set_objects(Some(targets))
                .build()
                .unwrap();

            let deleted = state
                .s3
                .delete_objects()
                .bucket(&repo.s3_bucket)
                .delete(delete)
                .send()
                .await;
            if let Err(err) = deleted {
                tracing::error!("Failed to delete objects: {err:?}");
            }
        }

        if res.is_truncated() == Some(true) {
            continuation_token = res.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }

    Ok(Json(DeleteDistributionResponse::default()))
}
