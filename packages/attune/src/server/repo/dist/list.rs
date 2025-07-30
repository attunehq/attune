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

/// A distribution within a package repository, representing a release channel or version.
///
/// Contains both required identifiers (name, suite, codename) and optional metadata
/// that provides additional context for package managers and users.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct Distribution {
    /// Unique database identifier for this distribution.
    pub id: i64,
    
    /// The distribution identifier that appears in the repository structure.
    /// For APT repositories, this appears in the URL path under `/dists/`.
    #[builder(into)]
    pub distribution: String,
    
    /// Human-readable description of this distribution.
    /// APT example: "Debian 11 (bullseye) - Stable Release"
    #[builder(into)]
    pub description: Option<String>,
    
    /// The organization or entity that produces this distribution.
    /// This appears in package manager output and helps users identify the source.
    /// Examples: "Debian", "Ubuntu", "ACME Corp"
    #[builder(into)]
    pub origin: Option<String>,
    
    /// A label for categorizing the distribution.
    /// Often the same as origin, but can differ for sub-projects or specialized channels.
    /// Examples: "Debian", "Debian-Security", "Ubuntu"
    #[builder(into)]
    pub label: Option<String>,
    
    /// The version number of this distribution release.
    /// APT examples: "11.0" for Debian 11, "22.04" for Ubuntu 22.04 LTS
    #[builder(into)]
    pub version: Option<String>,
    
    /// The suite name indicates the stability level or release channel.
    /// Common patterns include stability tiers (stable, testing, unstable) or
    /// update channels (release, updates, security).
    /// APT examples: "stable", "testing", "unstable", "oldstable", "experimental"
    #[builder(into)]
    pub suite: String,
    
    /// The codename is a unique identifier for a specific release version.
    /// This provides version stability - tools can reference a specific release
    /// regardless of its current stability status.
    /// APT examples: Debian uses "bullseye", "bookworm"; Ubuntu uses "focal", "jammy"
    #[builder(into)]
    pub codename: String,
}

/// Response containing all distributions within a repository.
///
/// Returns distributions sorted alphabetically by name. Each distribution
/// includes its complete metadata for display or further processing.
/// Empty repositories will return an empty array.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct ListDistributionsResponse {
    /// All distributions in the repository, sorted by distribution name.
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
