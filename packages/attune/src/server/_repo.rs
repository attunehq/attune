use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::types::time::OffsetDateTime;
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::{self, TenantID},
    server::ServerState,
};

#[derive(Serialize)]
pub struct Repository {
    id: i64,
    name: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateRepositoryRequest {
    name: String,
}

#[derive(Serialize)]
pub struct CreateRepositoryResponse {
    id: i64,
    name: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn create(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Json(req): Json<CreateRepositoryRequest>,
) -> Result<Json<CreateRepositoryResponse>, ErrorResponse> {
    let mut tx = state.db.begin().await.unwrap();

    // Find or create a repository with the given name. If a repository already
    // exists under a different tenant, abort.
    let existing = sqlx::query!(
        r#"
        SELECT id, name, tenant_id
        FROM debian_repository
        WHERE name = $1
        "#,
        req.name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    let repo = match existing {
        Some(existing) => {
            if existing.tenant_id != tenant_id.0 {
                return Err(ErrorResponse::new(
                    axum::http::StatusCode::NOT_FOUND,
                    "REPO_NOT_FOUND".to_string(),
                    "Repository not found".to_string(),
                ));
            }
            (existing.id, existing.name)
        }
        None => {
            let inserted = sqlx::query!(
                r#"
                INSERT INTO debian_repository (
                    tenant_id,
                    s3_bucket,
                    s3_prefix,
                    created_at,
                    updated_at
                )
                VALUES ($1, $2, $3, NOW(), NOW())
                RETURNING id, name
                "#,
                tenant_id.0,
                state.s3_bucket_name,
                format!(
                    "{}/{}",
                    tenant_id.0,
                    hex::encode(Sha256::digest(format!("{}/{}", tenant_id.0, req.name).as_bytes()))
                ),
            )
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            (inserted.id, inserted.name)
        }
    };

    tx.commit().await.unwrap();

    Ok(Json(CreateRepositoryResponse {
        id: repo.0,
        name: repo.1,
    }))
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn list(State(state): State<ServerState>, tenant_id: TenantID) -> Json<Vec<Repository>> {
    let repositories = sqlx::query!(
        r#"
            SELECT
                debian_repository_release.id,
                debian_repository.uri,
                debian_repository_release.distribution
            FROM debian_repository
                JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
            WHERE tenant_id = $1
            ORDER BY debian_repository_release.id ASC
        "#,
        tenant_id.0,
    )
    .fetch_all(&state.db)
    .await
    .unwrap();
    Json(
        repositories
            .into_iter()
            .map(|r| Repository {
                id: r.id,
                uri: r.uri,
                distribution: r.distribution,
            })
            .collect(),
    )
}

#[derive(Debug, Serialize)]
pub struct RepositoryStatus {
    changes: Vec<RepositoryChange>,
}

#[derive(Debug, Serialize)]
pub struct RepositoryChange {
    package_id: i64,
    component: String,
    package: String,
    version: String,
    architecture: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    change: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn status(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(release_id): Path<u64>,
) -> Result<Json<RepositoryStatus>, (axum::http::StatusCode, &'static str)> {
    // Check that the release exists and the user has permission to view it.
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    let changes = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.id,
            debian_repository_component.name AS component,
            debian_repository_package.package,
            debian_repository_package.version,
            debian_repository_package.architecture::TEXT AS "architecture!: String",
            debian_repository_package.staging_status::TEXT AS "staging_status!: String",
            debian_repository_package.updated_at
        FROM debian_repository_release
        JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
        JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE
            staging_status IS NOT NULL
            AND debian_repository_release.id = $1
        ORDER BY
            debian_repository_package.staging_status,
            debian_repository_package.updated_at DESC
        "#,
        release_id as i64,
    )
    .map(|row| RepositoryChange {
        package_id: row.id,
        component: row.component,
        package: row.package,
        version: row.version,
        architecture: row.architecture,
        updated_at: row.updated_at,
        change: row.staging_status
    })
    .fetch_all(&state.db)
    .await
    .unwrap();
    Ok(Json(RepositoryStatus { changes }))
}
