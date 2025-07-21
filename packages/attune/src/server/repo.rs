use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::types::time::OffsetDateTime;
use tracing::instrument;

use crate::{
    auth::{self, TenantID},
    server::ServerState,
};

#[derive(Serialize)]
pub struct Repository {
    id: i64,
    uri: String,
    distribution: String,
}

#[derive(Deserialize, Debug)]
pub struct CreateRepositoryRequest {
    uri: String,
    distribution: String,
    origin: Option<String>,
    label: Option<String>,
    version: Option<String>,
    suite: Option<String>,
    codename: Option<String>,
    description: Option<String>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn create(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<Json<Repository>, (axum::http::StatusCode, &'static str)> {
    let mut tx = state.db.begin().await.unwrap();

    // Note that this _actually_ creates a release _in addition to_ a
    // repository.
    //
    // TODO: Should we clarify the concept of "releases" distinct from
    // "repositories" in the UX?

    // Find or create a repository with the given URI. If a repository already
    // exists under a different tenant, abort.
    let existing = sqlx::query!(
        r#"
        SELECT id, uri, tenant_id
        FROM debian_repository
        WHERE uri = $1
        "#,
        payload.uri,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    let repo_id = match existing {
        Some(existing) => {
            if existing.tenant_id != tenant_id.0 {
                return Err((axum::http::StatusCode::NOT_FOUND, "Repository not found\n"));
            }
            existing.id
        }
        None => {
            sqlx::query!(
                r#"
                INSERT INTO debian_repository (
                    tenant_id,
                    uri,
                    s3_bucket,
                    s3_prefix,
                    created_at,
                    updated_at
                )
                VALUES ($1, $2, $3, $4, NOW(), NOW())
                RETURNING id
                "#,
                tenant_id.0,
                payload.uri,
                state.s3_bucket_name,
                format!(
                    "{}/{}",
                    tenant_id.0,
                    hex::encode(Sha256::digest(&payload.uri))
                ),
            )
            .fetch_one(&mut *tx)
            .await
            .unwrap()
            .id
        }
    };

    // Find or create a release with the given distribution.
    let existing = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository_release
        WHERE repository_id = $1
            AND distribution = $2
        "#,
        repo_id,
        payload.distribution,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    let release_id = match existing {
        Some(_) => {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                "Repository already exists\n",
            ));
        }
        None => {
            // Create the release.
            sqlx::query!(
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
                    fingerprint,
                    clearsigned,
                    detached,
                    updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW())
                RETURNING id
                "#,
                repo_id,
                payload.distribution,
                payload.description,
                payload.origin,
                payload.label,
                payload.version,
                payload.suite,
                payload.codename,
                "",
                hex::encode(Sha256::digest("").to_vec()),
                None::<String>,
                None::<String>,
            )
            .fetch_one(&mut *tx)
            .await
            .unwrap()
            .id
        }
    };

    tx.commit().await.unwrap();

    Ok(Json(Repository {
        id: release_id,
        uri: payload.uri,
        distribution: payload.distribution,
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
