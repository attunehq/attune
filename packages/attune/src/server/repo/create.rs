use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tracing::instrument;

use crate::{api::ErrorResponse, auth::TenantID, server::ServerState};

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
pub async fn handler(
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
                    hex::encode(Sha256::digest(
                        format!("{}/{}", tenant_id.0, req.name).as_bytes()
                    ))
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
