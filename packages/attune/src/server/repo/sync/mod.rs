pub mod check;
pub mod resync;

use aws_sdk_s3::types::ChecksumMode;
use base64::Engine;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::{Postgres, Transaction};
use tracing::{Level, instrument, trace};

use crate::api::{ErrorResponse, TenantID};

#[derive(Debug, Clone)]
pub enum Expected {
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

impl Expected {
    pub fn key(&self) -> &str {
        match self {
            Expected::Exists { key, .. } => key,
            Expected::DoesNotExist { key } => key,
        }
    }
}

/// Intended repository state given the current database state.
///
/// You should think of this as the "expected" state of the repository.
#[derive(Debug, Clone)]
pub struct RepositoryState {
    pub s3_bucket: String,
    pub release_contents: Expected,
    pub release_detachsigned: Expected,
    pub release_clearsigned: Expected,
    pub packages_indexes: Vec<Expected>,
    pub packages: Vec<Expected>,
}

/// S3 repository entities that are inconsistent with the current database
/// state.
///
/// You should think of this as the "actual" state of the repository.
///
/// Note that we track which entities are _inconsistent_ rather than which
/// entities are _consistent_, so we can explicitly list inconsistencies to fix
/// (rather than needing to infer which entities are inconsistent by comparing a
/// consistent set against a full set).
#[derive(Debug)]
pub struct InconsistentObjects {
    pub s3_bucket: String,
    pub release_contents: Option<Expected>,
    pub release_detachsigned: Option<Expected>,
    pub release_clearsigned: Option<Expected>,
    pub packages_indexes: Vec<Expected>,
    pub packages: Vec<Expected>,
}

#[instrument(level = Level::DEBUG, skip(tx))]
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
    let release_contents = Expected::Exists {
        key: format!("{}/dists/{}/Release", &repo.s3_prefix, &release_name),
        sha256sum: Sha256::digest(&release.contents).to_vec(),
        contents: release.contents,
    };
    let release_clearsigned = release
        .clearsigned
        .map(|clearsigned| Expected::Exists {
            key: format!("{}/dists/{}/InRelease", &repo.s3_prefix, &release_name),
            sha256sum: Sha256::digest(&clearsigned).to_vec(),
            contents: clearsigned,
        })
        .unwrap_or(Expected::DoesNotExist {
            key: format!("{}/dists/{}/InRelease", &repo.s3_prefix, &release_name),
        });
    let release_detachsigned = release
        .detached
        .map(|detached| Expected::Exists {
            key: format!("{}/dists/{}/Release.gpg", &repo.s3_prefix, &release_name),
            sha256sum: Sha256::digest(&detached).to_vec(),
            contents: detached,
        })
        .unwrap_or(Expected::DoesNotExist {
            key: format!("{}/dists/{}/Release.gpg", &repo.s3_prefix, &release_name),
        });

    // Check package indexes for consistency.
    let packages_indexes = sqlx::query!(r#"
        SELECT
            debian_repository_component.name AS "component",
            debian_repository_index_packages.architecture::TEXT AS "architecture!: String",
            debian_repository_index_packages.md5sum,
            debian_repository_index_packages.sha1sum,
            debian_repository_index_packages.sha256sum,
            debian_repository_index_packages.contents
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
        .flat_map(|packages_index| {
            let by_hash_prefix = format!(
                "{}/dists/{}/{}/binary-{}/by-hash",
                repo.s3_prefix,
                &release_name,
                &packages_index.component,
                &packages_index.architecture
            );
            let sha256sum = hex::decode(&packages_index.sha256sum)
                .expect("could not decode Packages index SHA256 sum");
            let contents = String::from_utf8(packages_index.contents).unwrap();
            [
                format!(
                    "{}/dists/{}/{}/binary-{}/Packages",
                    &repo.s3_prefix,
                    &release_name,
                    &packages_index.component,
                    &packages_index.architecture
                ),
                format!("{}/SHA256/{}", by_hash_prefix, packages_index.sha256sum),
                format!("{}/SHA1/{}", by_hash_prefix, packages_index.sha1sum),
                format!("{}/MD5Sum/{}", by_hash_prefix, packages_index.md5sum),
            ]
            .map(|key| Expected::Exists {
                key,
                sha256sum: sha256sum.clone(),
                contents: contents.clone(),
            })
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
        .map(|package| Expected::Exists {
            key: format!("{}/{}", repo.s3_prefix, package.filename),
            contents: format!("{}/packages/{}", package.s3_bucket, package.sha256sum),
            sha256sum: hex::decode(&package.sha256sum)
                .expect("could not decode package SHA256 sum"),
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

#[instrument(level = Level::DEBUG, skip(s3))]
async fn s3_object_consistent(
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    expected: &Expected,
) -> Result<bool, ErrorResponse> {
    Ok(match expected {
        Expected::Exists { key, sha256sum, .. } => s3
            .head_object()
            .bucket(s3_bucket)
            .key(key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .map(|head| {
                head.checksum_sha256()
                    .map(|checksum| {
                        let expected = base64::engine::general_purpose::STANDARD.encode(sha256sum);
                        trace!(actual = ?checksum, ?expected, "checking object sha256 checksum");
                        checksum == expected
                    })
                    .unwrap_or_else(|| {
                        trace!("could not read object sha256 checksum");
                        false
                    })
            })
            .unwrap_or_else(|err| {
                trace!(?err, "could not get object");
                false
            }),
        Expected::DoesNotExist { key } => s3
            .head_object()
            .bucket(s3_bucket)
            .key(key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .is_err_and(|err| err.into_service_error().is_not_found()),
    })
}

#[instrument(level = Level::DEBUG, skip(s3))]
pub async fn check_s3_consistency(
    s3: &aws_sdk_s3::Client,
    state: RepositoryState,
) -> Result<InconsistentObjects, ErrorResponse> {
    // Check release files for consistency.
    let release_contents =
        if s3_object_consistent(s3, &state.s3_bucket, &state.release_contents).await? {
            None
        } else {
            Some(state.release_contents)
        };
    let release_clearsigned =
        if s3_object_consistent(s3, &state.s3_bucket, &state.release_clearsigned).await? {
            None
        } else {
            Some(state.release_clearsigned)
        };
    let release_detachsigned =
        if s3_object_consistent(s3, &state.s3_bucket, &state.release_detachsigned).await? {
            None
        } else {
            Some(state.release_detachsigned)
        };

    // Check package indexes for consistency.
    let mut packages_indexes = Vec::new();
    for packages_index in state.packages_indexes {
        if !s3_object_consistent(s3, &state.s3_bucket, &packages_index).await? {
            packages_indexes.push(packages_index);
        }
    }

    // Check packages for consistency.
    let mut packages = Vec::new();
    for package in state.packages {
        if !s3_object_consistent(s3, &state.s3_bucket, &package).await? {
            packages.push(package);
        }
    }

    Ok(InconsistentObjects {
        s3_bucket: state.s3_bucket,
        release_contents,
        release_clearsigned,
        release_detachsigned,
        packages_indexes,
        packages,
    })
}

/// This Summary object is safe to serialize and send to clients, because it is
/// reasonably sized and doesn't leak implementation details (like S3 prefixes).
#[derive(Debug, Serialize, Deserialize)]
pub struct InconsistentSummary {
    pub release: bool,
    pub release_clearsigned: bool,
    pub release_detachsigned: bool,
    pub packages_indexes: Vec<String>,
    pub packages: Vec<String>,
}

impl From<&InconsistentObjects> for InconsistentSummary {
    fn from(inconsistent_objects: &InconsistentObjects) -> Self {
        Self {
            release: inconsistent_objects.release_contents.is_some(),
            release_clearsigned: inconsistent_objects.release_clearsigned.is_some(),
            release_detachsigned: inconsistent_objects.release_detachsigned.is_some(),
            packages_indexes: inconsistent_objects
                .packages_indexes
                .iter()
                .map(|pi| {
                    // Remove the S3 prefix to avoid leaking information.
                    let path = pi.key();
                    let (_, suffix) = path.split_once("/dists/").unwrap();
                    format!("dists/{suffix}")
                })
                .collect(),
            packages: inconsistent_objects
                .packages
                .iter()
                .map(|p| p.key().to_string())
                .collect(),
        }
    }
}
