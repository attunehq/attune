use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::ServerState,
};

#[derive(Serialize)]
pub struct Repository {
    id: i64,
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateRepositoryRequest {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateRepositoryResponse {
    pub id: i64,
    pub name: String,
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
    // exists, abort.
    let existing = sqlx::query!(
        r#"
        SELECT id, name
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        req.name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    if existing.is_some() {
        return Err(ErrorResponse::new(
            axum::http::StatusCode::BAD_REQUEST,
            "REPO_ALREADY_EXISTS".to_string(),
            "repository already exists".to_string(),
        ));
    }

    // Insert repository row.
    let inserted = sqlx::query!(
        r#"
        INSERT INTO debian_repository (
            name,
            tenant_id,
            s3_bucket,
            s3_prefix,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, NOW(), NOW())
        RETURNING id, name
        "#,
        req.name,
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

    tx.commit().await.unwrap();

    Ok(Json(CreateRepositoryResponse {
        id: inserted.id,
        name: inserted.name,
    }))
}
