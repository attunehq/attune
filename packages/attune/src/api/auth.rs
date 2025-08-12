//! Authentication and authorization.

use axum::{
    extract::{FromRef, FromRequestParts},
    http::request,
};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;

/// An extractor for tenants authenticated via API token.
#[derive(Debug, Clone, Copy)]
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
