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

#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct Distribution {
    pub id: i64,
    #[builder(into)]
    pub distribution: String,
    #[builder(into)]
    pub description: Option<String>,
    #[builder(into)]
    pub origin: Option<String>,
    #[builder(into)]
    pub label: Option<String>,
    #[builder(into)]
    pub version: Option<String>,
    #[builder(into)]
    pub suite: String,
    #[builder(into)]
    pub codename: String,
}

#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct ListDistributionsResponse {
    pub distributions: Vec<Distribution>,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
) -> Result<Json<ListDistributionsResponse>, ErrorResponse> {
    let repository_name = decode_repo_name(&repository_name)?;

    let repo = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        repository_name,
    )
    .fetch_optional(&state.db)
    .await
    .unwrap()
    .ok_or_else(|| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("REPO_NOT_FOUND")
            .message("repository not found")
            .build()
    })?;

    let distributions = sqlx::query!(
        r#"
        SELECT
            id,
            distribution,
            description,
            origin,
            label,
            version,
            suite,
            codename
        FROM debian_repository_release
        WHERE repository_id = $1
        ORDER BY distribution
        "#,
        repo.id,
    )
    .fetch_all(&state.db)
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        Distribution::builder()
            .id(row.id)
            .distribution(row.distribution)
            .suite(row.suite)
            .codename(row.codename)
            .maybe_description(row.description)
            .maybe_origin(row.origin)
            .maybe_label(row.label)
            .maybe_version(row.version)
            .build()
    })
    .collect();

    ListDistributionsResponse::builder()
        .distributions(distributions)
        .build()
        .pipe(Json)
        .pipe(Ok)
}
