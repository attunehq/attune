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

    // We need two separate queries to handle different deletion scenarios:
    //
    // 1. Standard Packages files: These exist for every component/architecture combination
    //    that has packages, regardless of whether an index was generated. We query through
    //    the packages table to ensure we delete all standard Packages files.
    //
    // 2. By-hash objects: These only exist when package indexes have been generated and
    //    uploaded with by-hash support. We need the exact hash values (md5sum, sha1sum, 
    //    sha256sum) from the index table to construct the precise S3 keys for deletion.
    //    Simply knowing component/architecture isn't enough - we need the hash values.
    //
    // These queries may return different result sets: packages might exist without indexes
    // (if index generation failed), or indexes might exist for different architectures
    // than expected due to package additions/removals over time.

    // Query all component/architecture combinations that have packages (for standard Packages files)
    let components_with_packages = sqlx::query!(
        r#"
        SELECT
            c.name as component_name,
            p.architecture::text as architecture
        FROM debian_repository_release r
        JOIN debian_repository_component c ON c.release_id = r.id
        JOIN debian_repository_component_package cp ON cp.component_id = c.id
        JOIN debian_repository_package p ON p.id = cp.package_id
        WHERE r.repository_id = $1 AND r.distribution = $2
        "#,
        repo.id,
        distribution_name,
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    // Query current package indexes with their hash values (for by-hash object deletion)
    let index_hashes = sqlx::query!(
        r#"
        SELECT
            c.name as component_name,
            i.architecture::text as architecture,
            i.md5sum,
            i.sha1sum,
            i.sha256sum
        FROM debian_repository_release r
        JOIN debian_repository_component c ON c.release_id = r.id
        JOIN debian_repository_index_packages i ON i.component_id = c.id
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

    // If no rows were affected, the distribution was already deleted or never existed.
    if result.rows_affected() == 0 {
        return Err(ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("DISTRIBUTION_NOT_FOUND")
            .message("distribution not found")
            .build());
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
        keys.extend(components_with_packages.iter().map(|record| {
            let prefix = format!(
                "{}/{}/binary-{}",
                prefix,
                record.component_name,
                record.architecture.as_ref().unwrap().replace('_', "-")
            );
            // TODO(#94): When compressed indexes are implemented, add their deletion here.
            format!("{prefix}/Packages")
        }));

        // Deletes by-hash objects.
        keys.extend(index_hashes.iter().flat_map(|record| {
            let arch = record.architecture.as_ref().unwrap().replace('_', "-");
            let by_hash_prefix = format!(
                "{}/{}/binary-{}/by-hash",
                prefix, record.component_name, arch
            );
            
            [
                format!("{}/SHA256/{}", by_hash_prefix, record.sha256sum),
                format!("{}/SHA1/{}", by_hash_prefix, record.sha1sum),
                format!("{}/MD5Sum/{}", by_hash_prefix, record.md5sum),
            ]
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
