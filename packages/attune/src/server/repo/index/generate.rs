use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            index::{PackageChange, generate_release_file_with_change},
        },
    },
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Repository {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GenerateIndexRequest {
    pub change: PackageChange,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GenerateIndexResponse {
    pub release: String,
    pub release_ts: OffsetDateTime,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repo_name): Path<String>,
    // FIXME: This is a GET request with a body.
    Json(req): Json<GenerateIndexRequest>,
) -> Result<Json<GenerateIndexResponse>, ErrorResponse> {
    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;
    if repo_name != req.change.repository {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "REPOSITORY_MISMATCH".to_string(),
            "repository name in path does not match repository name in request".to_string(),
        ));
    }

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    let release_ts = OffsetDateTime::now_utc();
    let result =
        generate_release_file_with_change(&mut tx, &tenant_id, &req.change, release_ts).await?;

    tx.commit().await.unwrap();

    Ok(Json(GenerateIndexResponse {
        release: result.release_file.contents,
        release_ts,
    }))
}
