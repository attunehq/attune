use axum::{
    Json,
    extract::{Path, State},
};
use bon::Builder;
use serde::{Deserialize, Serialize};
use tap::Pipe;
use tracing::instrument;

use crate::{api::ErrorResponse, auth::TenantID, server::ServerState};

use super::super::decode_repo_name;

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateDistributionRequest {
    pub name: String,
    pub description: Option<String>,
    pub origin: Option<String>,
    pub label: Option<String>,
    pub version: Option<String>,
    pub suite: String,
    pub codename: String,
}

#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct CreateDistributionResponse {
    pub id: i64,
    #[builder(into)]
    pub distribution: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
    Json(req): Json<CreateDistributionRequest>,
) -> Result<Json<CreateDistributionResponse>, ErrorResponse> {
    let repository_name = decode_repo_name(&repository_name)?;

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

    let existing = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
        "#,
        repo.id,
        req.name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    if existing.is_some() {
        return ErrorResponse::builder()
            .status(axum::http::StatusCode::BAD_REQUEST)
            .error("DIST_ALREADY_EXISTS")
            .message("distribution already exists")
            .build()
            .pipe(Err);
    }

    // Insert new distribution
    let inserted = sqlx::query!(
        r#"
        INSERT INTO debian_repository_release (
            repository_id,
            distribution,
            description,
            origin,
            label,
            version,
            suite,
            codename,
            contents,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '', NOW(), NOW())
        RETURNING id, distribution
        "#,
        repo.id,
        req.name,
        req.description,
        req.origin,
        req.label,
        req.version,
        req.suite,
        req.codename,
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    CreateDistributionResponse::builder()
        .id(inserted.id)
        .distribution(inserted.distribution)
        .build()
        .pipe(Json)
        .pipe(Ok)
}
