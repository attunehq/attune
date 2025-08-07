use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::{ErrorResponse, TenantID},
    server::ServerState,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageListParams {
    pub repository: Option<String>,
    pub distribution: Option<String>,
    pub component: Option<String>,

    pub name: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Package {
    pub repository: String,
    pub distribution: String,
    pub component: String,

    pub name: String,
    pub version: String,
    pub architecture: String,

    pub sha256sum: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageListResponse {
    pub packages: Vec<Package>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    params: Query<PackageListParams>,
) -> Result<Json<PackageListResponse>, ErrorResponse> {
    let packages = sqlx::query!(
        r#"
        SELECT
            debian_repository.name AS repository,
            debian_repository_release.distribution AS distribution,
            debian_repository_component.name AS component,

            debian_repository_package.package AS name,
            debian_repository_package.version,
            debian_repository_package.architecture::TEXT AS "architecture!: String",

            debian_repository_package.sha256sum
        FROM
            debian_repository_package
            JOIN debian_repository_component_package ON debian_repository_package.id = debian_repository_component_package.package_id
            JOIN debian_repository_component ON debian_repository_component_package.component_id = debian_repository_component.id
            JOIN debian_repository_release ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository ON debian_repository_release.repository_id = debian_repository.id
        WHERE
            debian_repository_package.tenant_id = $1
            AND (debian_repository.name = $2 OR $2 IS NULL)
            AND (debian_repository_release.distribution = $3 OR $3 IS NULL)
            AND (debian_repository_component.name = $4 OR $4 IS NULL)
            AND (debian_repository_package.package = $5 OR $5 IS NULL)
            AND (debian_repository_package.version = $6 OR $6 IS NULL)
            AND (debian_repository_package.architecture = $7::debian_repository_architecture OR $7 IS NULL)
        "#,
        tenant_id.0,
        // These explicit typecasts are necessary because otherwise Postgres
        // infers these argument types using the first callsite and assumes
        // these parameters are &str's.
        &params.repository as &Option<String>,
        &params.distribution as &Option<String>,
        &params.component as &Option<String>,
        &params.name as &Option<String>,
        &params.version as &Option<String>,
        &params.architecture as &Option<String>,
    )
    .fetch_all(&state.db)
    .await
    .unwrap()
    .into_iter()
    .map(|pkg| Package {
        repository: pkg.repository,
        distribution: pkg.distribution,
        component: pkg.component,
        name: pkg.name,
        version: pkg.version,
        architecture: pkg.architecture,
        sha256sum: pkg.sha256sum,
    })
    .collect::<Vec<_>>();

    Ok(Json(PackageListResponse { packages }))
}
