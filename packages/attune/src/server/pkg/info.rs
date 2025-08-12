use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::ServerState,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageInfoResponse {
    pub package: String,
    pub version: String,
    pub architecture: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(sha256sum): Path<String>,
) -> Result<Json<PackageInfoResponse>, ErrorResponse> {
    let pkg = sqlx::query!(
        r#"
        SELECT
            package,
            version,
            architecture::TEXT AS "architecture!: String"
        FROM debian_repository_package
        WHERE tenant_id = $1 AND sha256sum = $2
        LIMIT 1
        "#,
        tenant_id.0,
        sha256sum,
    )
    .fetch_optional(&state.db)
    .await
    .map_err(ErrorResponse::from)?;
    match pkg {
        Some(pkg) => Ok(Json(PackageInfoResponse {
            package: pkg.package,
            version: pkg.version,
            architecture: pkg.architecture,
        })),
        None => Err(ErrorResponse::new(
            StatusCode::NOT_FOUND,
            "PACKAGE_NOT_FOUND".to_string(),
            "package not found".to_string(),
        )),
    }
}
