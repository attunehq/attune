pub mod check;
pub mod resync;

use crate::api::ErrorResponse;
use crate::auth::TenantID;
use aws_sdk_s3::types::ChecksumMode;
use base64::Engine;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction};

/// S3 repository entities that are inconsistent with the current database
/// state.
///
/// Note that we track which entities are _inconsistent_ rather than which
/// entities are _consistent_, so we can explicitly list inconsistencies to fix
/// (rather than needing to infer which entities are inconsistent by comparing a
/// consistent set against a full set).
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

#[derive(Debug, Clone)]
pub struct RepositoryState {
    pub s3_bucket: String,
    pub release_contents: Expected,
    pub release_detachsigned: Expected,
    pub release_clearsigned: Expected,
    pub packages_indexes: Vec<Expected<(String, String)>>,
    pub packages: Vec<Expected<String>>,
}

#[derive(Debug, Clone)]
pub struct Expected<T = ()> {
    pub object: ExpectedObject,
    pub meta: T,
}

impl Expected {
    fn new(object: ExpectedObject) -> Self {
        Self { object, meta: () }
    }
}

#[derive(Debug, Clone)]
pub enum ExpectedObject {
    Exists {
        /// The S3 key of the object.
        key: String,
        /// The expected contents of the object, used to restore the object if it is
        /// inconsistent.
        ///
        /// For packages, this is the CopyObject key (which includes the bucket
        /// name) to the canonical package object.
        contents: String,
        /// The SHA256 sum of the object, used to determine whether the object has
        /// changed.
        sha256sum: Vec<u8>,
    },
    DoesNotExist {
        key: String,
    },
}

pub async fn query_repository_state(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &TenantID,
    repository_name: String,
    release_name: String,
) -> Result<RepositoryState, ErrorResponse> {
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
    let release_contents = Expected::new(ExpectedObject::Exists {
        key: format!("{}/dists/{}/Release", &repo.s3_prefix, &release_name),
        sha256sum: Sha256::digest(&release.contents).to_vec(),
        contents: release.contents,
    });
    let release_clearsigned = release
        .clearsigned
        .map(|clearsigned| {
            Expected::new(ExpectedObject::Exists {
                key: format!("{}/dists/{}/InRelease", &repo.s3_prefix, &release_name),
                sha256sum: Sha256::digest(&clearsigned).to_vec(),
                contents: clearsigned,
            })
        })
        .unwrap_or(Expected::new(ExpectedObject::DoesNotExist {
            key: format!("{}/dists/{}/InRelease", &repo.s3_prefix, &release_name),
        }));
    let release_detachsigned = release
        .detached
        .map(|detached| {
            Expected::new(ExpectedObject::Exists {
                key: format!("{}/dists/{}/Release.gpg", &repo.s3_prefix, &release_name),
                sha256sum: Sha256::digest(&detached).to_vec(),
                contents: detached,
            })
        })
        .unwrap_or(Expected::new(ExpectedObject::DoesNotExist {
            key: format!("{}/dists/{}/Release.gpg", &repo.s3_prefix, &release_name),
        }));

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
    let packages_indexes = packages_indexes
        .into_iter()
        .map(|packages_index| Expected {
            object: ExpectedObject::Exists {
                key: format!(
                    "{}/dists/{}/{}/binary-{}/Packages",
                    &repo.s3_prefix,
                    &release_name,
                    &packages_index.component,
                    &packages_index.architecture
                ),
                sha256sum: hex::decode(&packages_index.sha256sum)
                    .expect("could not decode Packages index SHA256 sum"),
                contents: packages_index.sha256sum,
            },
            meta: (packages_index.component, packages_index.architecture),
        })
        .collect::<Vec<_>>();

    // Check packages for consistency.
    let packages = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.s3_bucket,
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
    let packages = packages
        .into_iter()
        .map(|package| Expected {
            object: ExpectedObject::Exists {
                key: format!("{}/{}", repo.s3_prefix, package.filename),
                contents: format!("{}/packages/{}", package.s3_bucket, package.sha256sum),
                sha256sum: hex::decode(&package.sha256sum)
                    .expect("could not decode package SHA256 sum"),
            },
            meta: package.filename,
        })
        .collect::<Vec<_>>();

    Ok(RepositoryState {
        s3_bucket: repo.s3_bucket,
        release_contents,
        release_detachsigned,
        release_clearsigned,
        packages_indexes,
        packages,
    })
}

async fn s3_object_consistent<T>(
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    expected: &Expected<T>,
) -> Result<bool, ErrorResponse> {
    Ok(match &expected.object {
        ExpectedObject::Exists { key, sha256sum, .. } => s3
            .head_object()
            .bucket(s3_bucket)
            .key(key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .map(|head| {
                head.checksum_sha256()
                    .map(|checksum| {
                        checksum == base64::engine::general_purpose::STANDARD.encode(sha256sum)
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false),
        ExpectedObject::DoesNotExist { key } => s3
            .head_object()
            .bucket(s3_bucket)
            .key(key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .is_err_and(|err| err.into_service_error().is_not_found()),
    })
}

pub async fn check_s3_consistency(
    s3: aws_sdk_s3::Client,
    state: RepositoryState,
) -> Result<InconsistentObjects, ErrorResponse> {
    // Check release files for consistency.
    let inconsistent_release =
        !s3_object_consistent(&s3, &state.s3_bucket, &state.release_contents).await?;
    let inconsistent_clearsigned =
        !s3_object_consistent(&s3, &state.s3_bucket, &state.release_clearsigned).await?;
    let inconsistent_detachsigned =
        !s3_object_consistent(&s3, &state.s3_bucket, &state.release_detachsigned).await?;

    // Check package indexes for consistency.
    let mut inconsistent_packages_indexes = Vec::new();
    for packages_index in state.packages_indexes {
        if !s3_object_consistent(&s3, &state.s3_bucket, &packages_index).await? {
            inconsistent_packages_indexes.push(PackagesIndexId {
                component: packages_index.meta.0,
                architecture: packages_index.meta.1,
            });
        }
    }

    // Check packages for consistency.
    let mut inconsistent_packages = Vec::new();
    for package in state.packages {
        if !s3_object_consistent(&s3, &state.s3_bucket, &package).await? {
            inconsistent_packages.push(package.meta);
        }
    }

    Ok(InconsistentObjects {
        release: inconsistent_release,
        clearsigned: inconsistent_clearsigned,
        detached: inconsistent_detachsigned,
        packages_indexes: inconsistent_packages_indexes,
        package_filenames: inconsistent_packages,
    })
}

pub async fn resync_s3(
    s3: aws_sdk_s3::Client,
    state: RepositoryState,
) -> Result<(), ErrorResponse> {
    todo!()
}
