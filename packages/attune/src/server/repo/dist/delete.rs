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

    // Query components and their architectures for S3 cleanup
    let components = sqlx::query!(
        r#"
        SELECT
            c.name as component_name,
            pi.architecture::text as architecture
        FROM debian_repository_release r
        JOIN debian_repository_component c ON c.release_id = r.id
        JOIN debian_repository_index_packages pi ON pi.component_id = c.id
        WHERE r.repository_id = $1 AND r.distribution = $2
        "#,
        repo.id,
        distribution_name,
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    // Cascade will handle related records when deleting the distribution.
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

    // If no rows were affected, the distribution was already deleted.
    // We return an OK response here because the user's goal was accomplished regardless.
    if result.rows_affected() == 0 {
        return Ok(Json(DeleteDistributionResponse::default()));
    }

    // Find and delete orphaned packages; the returning clause is for S3 cleanup.
    let orphaned = sqlx::query!(
        r#"
        DELETE FROM debian_repository_package p
        WHERE p.tenant_id = $1
        AND NOT EXISTS (
            SELECT 1 FROM debian_repository_component_package cp
            WHERE cp.package_id = p.id
        )
        RETURNING p.id, p.s3_bucket, p.sha256sum
        "#,
        tenant_id.0,
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    // Database state is correct, so we can commit the transaction.
    // Now all we need to do is clean up S3 objects.
    tx.commit().await.unwrap();

    // Clean up S3 objects for this distribution based on known paths.
    let prefix = format!("{}/dists/{}", repo.s3_prefix, distribution_name);
    let keys = {
        // Deletes distribution metadata files.
        let mut keys = vec![
            format!("{prefix}/Release"),
            format!("{prefix}/Release.gpg"),
            format!("{prefix}/InRelease"),
        ];

        // Deletes component metadata files.
        keys.extend(components.iter().flat_map(|record| {
            let prefix = format!(
                "{}/{}/binary-{}",
                prefix,
                record.component_name,
                record.architecture.as_ref().unwrap().replace('_', "-")
            );
            // TODO: When compressed indexes are implemented, add their deletion here.
            [format!("{prefix}/Packages"), format!("{prefix}/Release")]
        }));

        // Deletes orphaned package files.
        keys.extend(
            orphaned
                .iter()
                .map(|pkg| format!("packages/{}", pkg.sha256sum)),
        );
        keys
    };

    // Delete all objects in batches.
    // TODO: make concurrent with `futures`' `BufferUnordered`.
    for chunk in keys.chunks(1000) {
        let objects = chunk
            .iter()
            .map(|key| {
                aws_sdk_s3::types::ObjectIdentifier::builder()
                    .key(key)
                    .build()
                    .unwrap()
            })
            .collect::<Vec<_>>();

        let delete = aws_sdk_s3::types::Delete::builder()
            .set_objects(Some(objects))
            .build()
            .unwrap();

        let result = state
            .s3
            .delete_objects()
            .bucket(&repo.s3_bucket)
            .delete(delete)
            .send()
            .await;

        if let Err(err) = result {
            tracing::error!("Failed to delete objects: {err:?}");
        }
    }

    Ok(Json(DeleteDistributionResponse::default()))
}
