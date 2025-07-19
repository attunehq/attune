//! Authentication and authorization.

use axum::{
    extract::{FromRef, FromRequestParts},
    http::request,
};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;

/// An extractor for tenants authenticated via API token.
#[derive(Debug, Clone)]
pub struct TenantID(pub i64);

fn parse_api_token(header: &axum::http::header::HeaderMap) -> Result<&str, &'static str> {
    let header = header
        .get("Authorization")
        .ok_or("`Authorization` header is missing")?;
    let token = header
        .to_str()
        .map_err(|_err| "`Authorization` header is malformed")?;
    let token = token
        .strip_prefix("Bearer ")
        .ok_or("`Authorization` scheme must be `Bearer`")?;
    Ok(token)
}

impl<S> FromRequestParts<S> for TenantID
where
    PgPool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (axum::http::StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let token = parse_api_token(&parts.headers)
            .map_err(|msg| (axum::http::StatusCode::UNAUTHORIZED, msg))?;
        let db = PgPool::from_ref(state);
        let tenant_id = sqlx::query!(
            r#"
            SELECT attune_tenant.id
            FROM attune_tenant
                JOIN attune_tenant_api_token ON attune_tenant_api_token.tenant_id = attune_tenant.id
            WHERE attune_tenant_api_token.token = $1;
            "#,
            Sha256::digest(token).as_slice().to_vec(),
        )
        .fetch_optional(&db)
        .await
        .map_err(|_err| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Could not validate API token",
            )
        })?;
        match tenant_id {
            Some(tenant_id) => Ok(TenantID(tenant_id.id)),
            None => Err((axum::http::StatusCode::UNAUTHORIZED, "Invalid API token\n")),
        }
    }
}

/// Returns an Error if the tenant does not own the release.
pub async fn tenant_owns_release(
    db: &PgPool,
    tenant_id: TenantID,
    release_id: u64,
) -> Result<(), (axum::http::StatusCode, &'static str)> {
    let release = sqlx::query!(
        r#"
            SELECT
                debian_repository_release.id,
                debian_repository.tenant_id
            FROM debian_repository
                JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
            WHERE debian_repository_release.id = $1
        "#,
        release_id as i64,
    )
    .fetch_optional(db)
    .await
    .unwrap();
    if let Some(release) = release {
        if release.tenant_id != tenant_id.0 {
            return Err((axum::http::StatusCode::NOT_FOUND, "Repository not found\n"));
        }
    } else {
        return Err((axum::http::StatusCode::NOT_FOUND, "Repository not found\n"));
    }
    Ok(())
}
