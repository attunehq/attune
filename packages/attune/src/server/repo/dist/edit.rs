use axum::{
    Json,
    extract::{Path, State},
};
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{
        ServerState,
        repo::{decode_repo_name, dist::decode_dist_name},
    },
};

/// Request to update metadata for an existing distribution.
///
/// Only the provided fields will be updated - omitted fields remain unchanged.
/// This allows partial updates without requiring all metadata to be resent.
/// Changes take effect immediately but may not be visible in repository indexes
/// until the next index generation.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct EditDistributionRequest {
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
    pub suite: Option<String>,

    /// The codename is a unique identifier for a specific release version.
    /// This provides version stability - tools can reference a specific release
    /// regardless of its current stability status.
    /// APT examples: Debian uses "bullseye", "bookworm"; Ubuntu uses "focal", "jammy"
    #[builder(into)]
    pub codename: Option<String>,
}

impl EditDistributionRequest {
    /// Check if any fields were provided.
    pub fn any_some(&self) -> bool {
        self.description.is_some()
            || self.origin.is_some()
            || self.label.is_some()
            || self.version.is_some()
            || self.suite.is_some()
            || self.codename.is_some()
    }
}

/// Response after successfully updating a distribution's metadata.
///
/// Returns the distribution ID and name for confirmation. The updated metadata
/// is immediately available through the API, though changes may not be reflected
/// in the repository indexes until the next index generation cycle.
#[derive(Serialize, Deserialize, Debug, Builder)]
pub struct EditDistributionResponse {
    /// Unique database identifier for this distribution.
    pub id: i64,

    /// The distribution name, confirming which distribution was updated.
    #[builder(into)]
    pub distribution: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((repository_name, distribution_name)): Path<(String, String)>,
    Json(req): Json<EditDistributionRequest>,
) -> Result<Json<EditDistributionResponse>, ErrorResponse> {
    let repository_name = decode_repo_name(&repository_name)?;
    let distribution_name = decode_dist_name(&distribution_name)?;

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

    let dist = sqlx::query!(
        r#"
        SELECT id, distribution, description, origin, label, version, suite, codename
        FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
        "#,
        repo.id,
        distribution_name,
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    .ok_or_else(|| {
        ErrorResponse::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .error("DIST_NOT_FOUND")
            .message("distribution not found")
            .build()
    })?;

    // Update the distribution with provided fields
    let updated = sqlx::query!(
        r#"
        UPDATE debian_repository_release
        SET
            description = COALESCE($3, description),
            origin = COALESCE($4, origin),
            label = COALESCE($5, label),
            version = COALESCE($6, version),
            suite = COALESCE($7, suite),
            codename = COALESCE($8, codename),
            updated_at = NOW()
        WHERE id = $1 AND repository_id = $2
        RETURNING id, distribution
        "#,
        dist.id,
        repo.id,
        req.description.or(dist.description),
        req.origin.or(dist.origin),
        req.label.or(dist.label),
        req.version.or(dist.version),
        req.suite.or(Some(dist.suite)),
        req.codename.or(Some(dist.codename)),
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    Ok(Json(
        EditDistributionResponse::builder()
            .id(updated.id)
            .distribution(updated.distribution)
            .build(),
    ))
}
