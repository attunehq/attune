use aws_sdk_s3::types::ChecksumAlgorithm;
use axum::{
    Json,
    extract::{Path, State},
};
use base64::Engine;
use md5::{Digest as _, Md5};
use serde::{Deserialize, Serialize};
use tracing::{Level, debug, instrument};

use crate::{
    api::{ErrorResponse, TenantID},
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            sync::{
                Expected, InconsistentObjects, InconsistentSummary, check_s3_consistency,
                query_repository_state,
            },
        },
    },
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ResyncRepositoryResponse {
    #[serde(flatten)]
    pub status: InconsistentSummary,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repo_name, release_name)): Path<(String, String)>,
) -> Result<Json<ResyncRepositoryResponse>, ErrorResponse> {
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

    // Resync inconsistent objects.
    Ok(Json(resync_s3(&state.s3, inconsistent_objects).await?))
}

#[instrument(level = Level::DEBUG, skip(s3))]
async fn resync_index(
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    expected: Expected,
) -> Result<(), ErrorResponse> {
    match expected {
        Expected::Exists {
            key,
            sha256sum,
            contents,
        } => {
            s3.put_object()
                .bucket(s3_bucket)
                .key(key)
                .content_md5(
                    base64::engine::general_purpose::STANDARD
                        .encode(Md5::digest(contents.as_bytes())),
                )
                .checksum_algorithm(ChecksumAlgorithm::Sha256)
                .checksum_sha256(base64::engine::general_purpose::STANDARD.encode(sha256sum))
                .body(contents.as_bytes().to_vec().into())
                .send()
                .await
                .unwrap();
        }
        Expected::DoesNotExist { key } => {
            s3.delete_object()
                .bucket(s3_bucket)
                .key(key)
                .send()
                .await
                .unwrap();
        }
    }
    Ok(())
}

/// Like `resync_index`, but for packages (which are copied from their canonical
/// location, rather than uploaded directly).
#[instrument(level = Level::DEBUG, skip(s3))]
async fn resync_package(
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    expected: Expected,
) -> Result<(), ErrorResponse> {
    match expected {
        Expected::Exists { key, contents, .. } => {
            s3.copy_object()
                .bucket(s3_bucket)
                .key(key)
                .copy_source(contents)
                .send()
                .await
                .unwrap();
        }
        Expected::DoesNotExist { key } => {
            s3.delete_object()
                .bucket(s3_bucket)
                .key(key)
                .send()
                .await
                .unwrap();
        }
    }
    Ok(())
}

#[instrument(level = Level::DEBUG, skip(s3))]
pub async fn resync_s3(
    s3: &aws_sdk_s3::Client,
    inconsistent_objects: InconsistentObjects,
) -> Result<ResyncRepositoryResponse, ErrorResponse> {
    let status = InconsistentSummary::from(&inconsistent_objects);
    let s3_bucket = inconsistent_objects.s3_bucket;
    if let Some(release_contents) = inconsistent_objects.release_contents {
        resync_index(s3, &s3_bucket, release_contents).await?;
    }
    if let Some(release_clearsigned) = inconsistent_objects.release_clearsigned {
        resync_index(s3, &s3_bucket, release_clearsigned).await?;
    }
    if let Some(release_detachsigned) = inconsistent_objects.release_detachsigned {
        resync_index(s3, &s3_bucket, release_detachsigned).await?;
    }
    for packages_index in inconsistent_objects.packages_indexes {
        resync_index(s3, &s3_bucket, packages_index).await?;
    }
    for package in inconsistent_objects.packages {
        resync_package(s3, &s3_bucket, package).await?;
    }
    Ok(ResyncRepositoryResponse { status })
}
