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

#[derive(Serialize, Deserialize, Debug)]
pub struct EditDistributionRequest {
    pub description: Option<String>,
    pub origin: Option<String>,
    pub label: Option<String>,
    pub version: Option<String>,
    pub suite: Option<String>,
    pub codename: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct EditDistributionResponse {
    pub id: i64,
    #[builder(into)]
    pub distribution: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repository_name, distribution_name)): Path<(String, String)>,
    Json(req): Json<EditDistributionRequest>,
) -> Result<Json<EditDistributionResponse>, ErrorResponse> {
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

    let dist = sqlx::query!(
        r#"
        SELECT id, distribution, description, origin, label, version, suite, codename
        FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
        "#,
        repo.id,
        distribution_name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .ok_or_else(|| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("DIST_NOT_FOUND")
            .message("distribution not found")
            .build()
    })?;

    // Update the distribution with provided fields
    let updated = sqlx::query!(
        r#"
        UPDATE debian_repository_release
        SET
            description = COALESCE($3, description),
            origin = COALESCE($4, origin),
            label = COALESCE($5, label),
            version = COALESCE($6, version),
            suite = COALESCE($7, suite),
            codename = COALESCE($8, codename),
            updated_at = NOW()
        WHERE id = $1 AND repository_id = $2
        RETURNING id, distribution
        "#,
        dist.id,
        repo.id,
        req.description.or(dist.description),
        req.origin.or(dist.origin),
        req.label.or(dist.label),
        req.version.or(dist.version),
        req.suite.or(Some(dist.suite)),
        req.codename.or(Some(dist.codename)),
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    EditDistributionResponse::builder()
        .id(updated.id)
        .distribution(updated.distribution)
        .build()
        .pipe(Json)
        .pipe(Ok)
}