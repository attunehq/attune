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

/// Request to create a new distribution (release) within a package repository.
///
/// A distribution represents a specific release channel or version of packages,
/// serving as a primary organizational structure. Each distribution can contain
/// multiple components or sections for categorizing packages.
///
/// While currently implemented for APT repositories, the concept generalizes
/// across package management systems (e.g., YUM repositories have similar concepts).
///
/// Example in apt sources.list:
/// `deb https://example.com/debian bookworm main contrib`
/// where "bookworm" is the distribution name.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct CreateDistributionRequest {
    /// The distribution identifier that will appear in the repository structure.
    /// For APT repositories, this appears in the URL path under `/dists/`.
    /// This is typically either the suite name (e.g., "stable") or codename (e.g., "bullseye").
    /// Example usage: `deb https://example.com/debian {name} main`
    #[builder(into)]
    pub name: String,

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
}

/// Response after successfully creating a new distribution.
///
/// Returns the assigned database ID and the distribution name for confirmation.
/// The distribution is immediately available for adding packages, though it won't
/// appear in the repository until packages are added and the repository index is generated.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct CreateDistributionResponse {
    /// Unique database identifier for this distribution.
    /// Use this ID for subsequent operations like editing or deleting the distribution.
    pub id: i64,

    /// The distribution name as stored, matching the request.
    /// This confirms the exact identifier that will appear in the repository structure.
    #[builder(into)]
    pub distribution: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repository_name): Path<String>,
    Json(req): Json<CreateDistributionRequest>,
) -> Result<Json<CreateDistributionResponse>, ErrorResponse> {
    let repository_name = decode_repo_name(&repository_name)?;

    let mut tx = state.db.begin().await.unwrap();
    let repo = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        repository_name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .ok_or_else(|| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("REPO_NOT_FOUND")
            .message("repository not found")
            .build()
    })?;

    let existing = sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
        "#,
        repo.id,
        req.name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap();
    if existing.is_some() {
        return ErrorResponse::builder()
            .status(axum::http::StatusCode::BAD_REQUEST)
            .error("DIST_ALREADY_EXISTS")
            .message("distribution already exists")
            .build()
            .pipe(Err);
    }

    // Insert new distribution
    let inserted = sqlx::query!(
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
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '', NOW(), NOW())
        RETURNING id, distribution
        "#,
        repo.id,
        req.name,
        req.description,
        req.origin,
        req.label,
        req.version,
        req.suite,
        req.codename,
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    CreateDistributionResponse::builder()
        .id(inserted.id)
        .distribution(inserted.distribution)
        .build()
        .pipe(Json)
        .pipe(Ok)
}
