pub mod check;
pub mod resync;

use aws_sdk_s3::types::ChecksumMode;
use base64::Engine;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction};

use crate::api::{ErrorResponse, TenantID};

#[derive(Debug, Serialize, Deserialize)]
pub struct InconsistentObjects {
    release: bool,
    clearsigned: bool,
    detached: bool,
    packages_indexes: Vec<PackagesIndexId>,
    package_filenames: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagesIndexId {
    component: String,
    architecture: String,
}

async fn check_consistency(
    tx: &mut Transaction<'_, Postgres>,
    s3: aws_sdk_s3::Client,
    tenant_id: &TenantID,
    repository_name: String,
    release_name: String,
) -> Result<InconsistentObjects, ErrorResponse> {
    let repo = sqlx::query!(
        r#"
        SELECT id, name, s3_bucket, s3_prefix
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
    "#,
        tenant_id.0,
        repository_name
    )
    .fetch_optional(&mut **tx)
    .await
    .unwrap()
    .ok_or(ErrorResponse::new(
        StatusCode::NOT_FOUND,
        "REPOSITORY_NOT_FOUND".to_string(),
        "repository not found".to_string(),
    ))?;

    let release = sqlx::query!(
        r#"
        SELECT id, contents, clearsigned, detached
        FROM debian_repository_release
        WHERE repository_id = $1 AND distribution = $2
    "#,
        &repo.id,
        &release_name
    )
    .fetch_optional(&mut **tx)
    .await
    .unwrap()
    .ok_or(ErrorResponse::new(
        StatusCode::NOT_FOUND,
        "RELEASE_NOT_FOUND".to_string(),
        "release not found".to_string(),
    ))?;

    // Check release files for consistency.
    let inconsistent_release = s3
        .head_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release",
            repo.s3_prefix, &release_name
        ))
        .checksum_mode(ChecksumMode::Enabled)
        .send()
        .await
        .map(|head| {
            head.checksum_sha256()
                .map(|checksum| {
                    checksum
                        != base64::engine::general_purpose::STANDARD
                            .encode(Sha256::digest(&release.contents))
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);
    let inconsistent_clearsigned = s3
        .head_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/InRelease",
            repo.s3_prefix, &release_name
        ))
        .checksum_mode(ChecksumMode::Enabled)
        .send()
        .await
        .map(|head| {
            head.checksum_sha256()
                .map(|checksum| {
                    release
                        .clearsigned
                        .map(|clearsigned| {
                            checksum
                                != base64::engine::general_purpose::STANDARD
                                    .encode(Sha256::digest(&clearsigned))
                        })
                        .unwrap_or(true)
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);
    let inconsistent_detached = s3
        .head_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release.gpg",
            repo.s3_prefix, &release_name
        ))
        .checksum_mode(ChecksumMode::Enabled)
        .send()
        .await
        .map(|head| {
            head.checksum_sha256()
                .map(|checksum| {
                    release
                        .detached
                        .map(|detached| {
                            checksum
                                != base64::engine::general_purpose::STANDARD
                                    .encode(Sha256::digest(&detached))
                        })
                        .unwrap_or(true)
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);

    // Check package indexes for consistency.
    let packages_indexes = sqlx::query!(r#"
        SELECT
            debian_repository_component.name AS "component",
            debian_repository_index_packages.architecture::TEXT AS "architecture!: String",
            debian_repository_index_packages.sha256sum
        FROM
            debian_repository_index_packages
            JOIN debian_repository_component ON debian_repository_index_packages.component_id = debian_repository_component.id
        WHERE
            debian_repository_component.release_id = $1
    "#,
        &release.id,
    )
    .fetch_all(&mut **tx)
    .await
    .unwrap();
    let mut inconsistent_packages_indexes = Vec::new();
    for packages_index in packages_indexes {
        if s3
            .head_object()
            .bucket(&repo.s3_bucket)
            .key(format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix,
                &release_name,
                packages_index.component,
                packages_index.architecture
            ))
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .map(|head| {
                head.checksum_sha256()
                    .map(|checksum| {
                        let expected = hex::decode(&packages_index.sha256sum)
                            .expect("could not decode Packages index SHA256 sum");
                        let expected = base64::engine::general_purpose::STANDARD.encode(expected);
                        checksum != expected
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true)
        {
            inconsistent_packages_indexes.push(PackagesIndexId {
                component: packages_index.component,
                architecture: packages_index.architecture,
            });
        }
    }

    // Check packages for consistency.
    let packages = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.sha256sum,
            debian_repository_component_package.filename
        FROM
            debian_repository_package
            JOIN debian_repository_component_package ON debian_repository_package.id = debian_repository_component_package.package_id
            JOIN debian_repository_component ON debian_repository_component_package.component_id = debian_repository_component.id
        WHERE
            debian_repository_component.release_id = $1
        "#,
        &release.id,
    )
    .fetch_all(&mut **tx)
    .await
    .unwrap();
    let mut inconsistent_packages = Vec::new();
    for package in packages {
        if s3
            .head_object()
            .bucket(&repo.s3_bucket)
            .key(format!("{}/{}", repo.s3_prefix, package.filename))
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .map(|head| {
                head.checksum_sha256()
                    .map(|checksum| {
                        let expected = hex::decode(&package.sha256sum)
                            .expect("could not decode package SHA256 sum");
                        let expected = base64::engine::general_purpose::STANDARD.encode(expected);
                        checksum != expected
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(true)
        {
            inconsistent_packages.push(package.filename);
        }
    }

    Ok(InconsistentObjects {
        release: inconsistent_release,
        clearsigned: inconsistent_clearsigned,
        detached: inconsistent_detached,
        packages_indexes: inconsistent_packages_indexes,
        package_filenames: inconsistent_packages,
    })
}
