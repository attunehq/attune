use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{ServerState, repo::decode_repo_name, repo::index::generate::GenerateIndexRequest},
};

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexRequest {
    pub diff: GenerateIndexRequest,
    pub clearsigned: String,
    pub detachsigned: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum IndexChange {
    Add,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexResponse {}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
    Json(req): Json<SignIndexRequest>,
) -> Result<Json<SignIndexResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repository_name = decode_repo_name(&repository_name)?;

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Load the package to be either added or removed. If it does not exist,
    // return an error.
    let changed_package = match sqlx::query!(
        r#"
            SELECT
                paragraph,
                package,
                version,
                architecture::TEXT AS "architecture!: String",
                size,
                md5sum,
                sha1sum,
                sha256sum
            FROM debian_repository_package
            WHERE
                tenant_id = $1
                AND sha256sum = $2
        "#,
        tenant_id.0,
        req.diff.package_sha256sum
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    {
        Some(pkg) => pkg,
        None => {
            return Err(ErrorResponse::new(
                StatusCode::NOT_FOUND,
                "PACKAGE_NOT_FOUND".to_string(),
                "package not found".to_string(),
            ));
        }
    };

    // Load the repository. If it does not exist, return an error.
    let repo = match sqlx::query!(r#"
        SELECT id, name
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        req.diff.repository
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    {
        Some(repo) => repo,
        None => {
            return Err(ErrorResponse::new(
                StatusCode::NOT_FOUND,
                "REPOSITORY_NOT_FOUND".to_string(),
                "repository not found".to_string(),
            ));
        }
    };

    // Replay the diff onto the current state of the index. Since index
    // generation is deterministic, this should yield the same index that was
    // signed locally.

    // Compare the replayed index with the signed index. Accept the signature if
    // the index contents match. Otherwise, return an error.

    // Save the new index state to the database.

    // Commit the transaction. At this point, the transaction may abort because
    // of a concurrent index change. This should trigger the handler to retry.

    // Save the new index state to S3. This must occur after the transaction
    // commits so that we are sure that we are not incorrectly overwriting a
    // concurrent index update.
    //
    // Note that there is a small risk of a handler crash _after_ the
    // transaction commits and _before_ the index update in S3 completes. This
    // will leave the repository in an inconsistent state, but can be fixed by
    // redoing the S3 upload from database state.

    Ok(Json(SignIndexResponse {}))
}
