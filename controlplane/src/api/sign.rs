use std::{collections::HashSet, io::Write};

use crate::api::ServerState;
use crate::auth::{self, TenantID};
use axum::{
    Json,
    extract::{Path, State},
};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tabwriter::TabWriter;
use time::{OffsetDateTime, format_description::well_known::Rfc2822};
use tracing::instrument;

#[derive(Debug, Serialize)]
pub struct ReleaseFile {
    release: String,
    fingerprint: String,
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn generate_indexes(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(release_id): Path<u64>,
) -> Result<Json<ReleaseFile>, (axum::http::StatusCode, &'static str)> {
    // TODO: Maybe we should generate indexes incrementally on package add in
    // the background, rather than doing a full generation on every sync? That
    // might be a little faster.

    // Check that the release exists and the user has permission to sign it.
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    // TODO: Do some optimization to check for staleness to avoid regenerating
    // indexes every call if packages haven't changed.

    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    let repo = sqlx::query!(
        r#"
        SELECT
            debian_repository_release.distribution,
            debian_repository_release.origin,
            debian_repository_release.label,
            debian_repository_release.version,
            debian_repository_release.suite,
            debian_repository_release.codename,
            debian_repository_release.description,
            debian_repository.s3_bucket,
            debian_repository.s3_prefix
        FROM debian_repository_release
            JOIN debian_repository ON debian_repository.id = debian_repository_release.repository_id
        WHERE debian_repository_release.id = $1
        "#,
        release_id as i64
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Generate package indexes, save to database, and upload to staging.
    let package_indexes = sqlx::query!(r#"
        SELECT DISTINCT
            debian_repository_package.component_id,
            debian_repository_component.name AS component,
            debian_repository_package.architecture::TEXT AS "architecture!: String"
        FROM debian_repository_release
        JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
        JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE debian_repository_release.id = $1
    "#, release_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for package_index in &package_indexes {
        // For each package index file, generate the index.
        let pkgs = sqlx::query!(
            r#"
            SELECT
                paragraph,
                filename,
                size,
                md5sum,
                sha1sum,
                sha256sum
            FROM debian_repository_package
            WHERE
                component_id = $1
                AND architecture = $2::debian_repository_architecture
        "#,
            package_index.component_id,
            package_index.architecture as _
        )
        .fetch_all(&mut *tx)
        .await
        .unwrap();

        let index = {
            let mut index = pkgs
                .into_iter()
                .map(|mut pkg| {
                    let fields = pkg
                        .paragraph
                        .as_object_mut()
                        .unwrap()
                        .into_iter()
                        .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap()))
                        .chain(vec![
                            format!("Filename: {}", pkg.filename),
                            format!("Size: {}", pkg.size.to_string()),
                            format!("MD5sum: {}", pkg.md5sum),
                            format!("SHA1: {}", pkg.sha1sum),
                            format!("SHA256: {}", pkg.sha256sum),
                        ])
                        .collect::<Vec<String>>();
                    fields.join("\n")
                })
                .collect::<Vec<String>>()
                .join("\n\n");
            index.push('\n');
            index
        };

        // Compute hashes.
        let sha256sum = hex::encode(Sha256::digest(&index));
        let sha1sum = hex::encode(Sha1::digest(&index));
        let md5sum = hex::encode(Md5::digest(&index));
        let size = index.len() as i64;

        // Save index to database.
        sqlx::query!(
            r#"
            INSERT INTO debian_repository_index_packages (
                component_id,
                architecture,

                compression,
                size,
                contents,
                md5sum,
                sha1sum,
                sha256sum,

                updated_at
            ) VALUES ($1, $2::debian_repository_architecture, NULL, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (component_id, architecture)
            DO UPDATE SET
                component_id = $1,
                architecture = $2::debian_repository_architecture,
                compression = NULL,
                size = $3,
                contents = $4,
                md5sum = $5,
                sha1sum = $6,
                sha256sum = $7,
                updated_at = NOW()
            "#,
            package_index.component_id,
            package_index.architecture as _,
            // compression = NULL,
            size,
            index.as_bytes(),
            md5sum,
            sha1sum,
            sha256sum
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        // Upload index to staging.
        state
            .s3
            .put_object()
            .bucket(&repo.s3_bucket)
            .key(format!(
                "{}/staging/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix,
                repo.distribution,
                package_index.component,
                package_index.architecture
            ))
            .body(axum::body::Bytes::from_owner(index).into())
            .send()
            .await
            .unwrap();
    }

    // Generate release file, save to database, and upload to staging.
    //
    // Note that the date format is RFC 2822. _Technically_, the Debian spec
    // says it should be the date format of `date -R -u`, which technically is
    // RFC 5322, but these formats are compatible. 5322 is a later revision of
    // 2822 that retains backwards compatibility.
    let release_file = {
        let date = OffsetDateTime::now_utc().format(&Rfc2822).unwrap();
        let mut arch_set = HashSet::new();
        let mut comp_set = HashSet::new();
        for p in package_indexes {
            arch_set.insert(p.architecture);
            comp_set.insert(p.component);
        }
        let archs = arch_set
            .into_iter()
            .fold(String::new(), |acc_archs, arch| acc_archs + " " + &arch);
        let comps = comp_set
            .into_iter()
            .fold(String::new(), |acc_comps, comp| acc_comps + " " + &comp);
        let release_fields: Vec<(&str, Option<String>)> = vec![
            ("Origin", repo.origin.clone()),
            ("Label", repo.label.clone()),
            ("Version", repo.version.clone()),
            ("Suite", Some(repo.suite.clone())),
            ("Codename", Some(repo.codename.clone())),
            ("Date", Some(date)),
            ("Architectures", Some(archs)),
            ("Components", Some(comps)),
            ("Description", repo.description.clone()),
        ];
        let mut release_file = String::new();
        for (k, v) in release_fields {
            if let Some(v) = v {
                release_file.push_str(&format!("{}: {}\n", k, v));
            }
        }

        let indexes = sqlx::query!(r#"
            SELECT
                debian_repository_component.name AS component,
                debian_repository_index_packages.architecture::TEXT AS "architecture!: String",
                debian_repository_index_packages.size,
                debian_repository_index_packages.md5sum,
                debian_repository_index_packages.sha1sum,
                debian_repository_index_packages.sha256sum
            FROM debian_repository_release
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_index_packages ON debian_repository_index_packages.component_id = debian_repository_component.id
            WHERE
                debian_repository_release.id = $1
            "#,
            release_id as i64
        )
        .fetch_all(&mut *tx)
        .await
        .unwrap();

        release_file = release_file + "MD5Sum:\n";
        let mut md5writer = TabWriter::new(vec![]);
        for index in &indexes {
            // TODO: Handle compressed indexes.
            write!(
                &mut md5writer,
                " {}\t{} {}\n",
                index.md5sum,
                index.size,
                format!("{}/binary-{}/Packages", index.component, index.architecture)
            )
            .unwrap();
        }
        md5writer.flush().unwrap();
        release_file = release_file + &String::from_utf8(md5writer.into_inner().unwrap()).unwrap();

        release_file = release_file + "SHA256:\n";
        let mut sha256writer = TabWriter::new(vec![]);
        for index in &indexes {
            // TODO: Handle compressed indexes.
            write!(
                &mut sha256writer,
                " {}\t{} {}\n",
                index.sha256sum,
                index.size,
                format!("{}/binary-{}/Packages", index.component, index.architecture)
            )
            .unwrap();
        }
        sha256writer.flush().unwrap();
        release_file =
            release_file + &String::from_utf8(sha256writer.into_inner().unwrap()).unwrap();
        release_file
    };
    let release_file_fingerprint = hex::encode(Sha256::digest(release_file.as_bytes()).as_slice());

    // Save release file to database.
    sqlx::query!(
        r#"
        UPDATE debian_repository_release
        SET
            description = $1,
            origin = $2,
            label = $3,
            version = $4,
            suite = $5,
            codename = $6,
            contents = $7,
            fingerprint = $8,
            updated_at = NOW()
        WHERE id = $9
        "#,
        repo.description,
        repo.origin,
        repo.label,
        repo.version,
        repo.suite,
        repo.codename,
        release_file,
        release_file_fingerprint,
        release_id as i64
    )
    .execute(&mut *tx)
    .await
    .unwrap();

    // Upload release file to staging.
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/staging/dists/{}/Release",
            repo.s3_prefix, repo.distribution
        ))
        .body(axum::body::Bytes::from_owner(release_file.clone()).into())
        .send()
        .await
        .unwrap();

    tx.commit().await.unwrap();

    // Return generated release file.
    Ok(Json(ReleaseFile {
        release: release_file,
        fingerprint: release_file_fingerprint,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SyncRepositoryRequest {
    fingerprint: String,
    clearsigned: String,
    detached: String,
}

#[axum::debug_handler]
#[instrument(skip(state, payload))]
pub async fn sync_repository(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(release_id): Path<u64>,
    Json(payload): Json<SyncRepositoryRequest>,
) -> Result<(), (axum::http::StatusCode, &'static str)> {
    // Check that the release exists and the user has permission to sign it.
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    // TODO: Check that signatures and checksums are actually valid and
    // up-to-date?

    // Save signatures to database.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let repo = sqlx::query!(
        r#"
            SELECT
                debian_repository_release.distribution,
                debian_repository_release.fingerprint,
                debian_repository.s3_bucket,
                debian_repository.s3_prefix
            FROM debian_repository_release
            JOIN debian_repository ON debian_repository.id = debian_repository_release.repository_id
            WHERE debian_repository_release.id = $1
        "#,
        release_id as i64
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    if repo.fingerprint != payload.fingerprint {
        return Err((axum::http::StatusCode::BAD_REQUEST, "Invalid fingerprint"));
    }

    let release_file = sqlx::query!(
        r#"
            UPDATE debian_repository_release
            SET clearsigned = $1, detached = $2, updated_at = NOW()
            WHERE id = $3
            RETURNING contents
        "#,
        payload.clearsigned,
        payload.detached,
        release_id as i64,
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    // Copy new package files from staging to active.
    let added_packages = sqlx::query!(
        r#"
        SELECT filename
        FROM debian_repository_release
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE
            staging_status = 'add'
            AND repository_id = $1
        "#,
        release_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for added in added_packages {
        state
            .s3
            .copy_object()
            .copy_source(format!(
                "{}/{}/staging/{}",
                repo.s3_bucket, repo.s3_prefix, added.filename
            ))
            .bucket(&repo.s3_bucket)
            .key(format!("{}/{}", repo.s3_prefix, added.filename))
            .send()
            .await
            .unwrap();
    }

    // Copy package indexes from staging to active.
    //
    // TODO: It's a little sketchy that we just copy files from staging to
    // active without verifying that they're still the same correct file.
    // Ideally, we'd probably generate them from scratch during this step.
    let package_indexes = sqlx::query!(
        r#"
        SELECT
            debian_repository_component.name AS component,
            debian_repository_index_packages.architecture::TEXT AS "architecture!: String"
        FROM debian_repository_release
        JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
        JOIN debian_repository_index_packages ON debian_repository_index_packages.component_id = debian_repository_component.id
        WHERE
            debian_repository_release.id = $1
        "#,
        release_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    for index in package_indexes {
        state
            .s3
            .copy_object()
            .copy_source(format!(
                "{}/{}/staging/dists/{}/{}/binary-{}/Packages",
                repo.s3_bucket,
                repo.s3_prefix,
                repo.distribution,
                index.component,
                index.architecture
            ))
            .bucket(&repo.s3_bucket)
            .key(format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix, repo.distribution, index.component, index.architecture
            ))
            .send()
            .await
            .unwrap();
    }

    // Upload release files.
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release",
            repo.s3_prefix, repo.distribution
        ))
        .body(axum::body::Bytes::from_owner(release_file.contents).into())
        .send()
        .await
        .unwrap();

    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release.gpg",
            repo.s3_prefix, repo.distribution
        ))
        .body(axum::body::Bytes::from_owner(payload.detached).into())
        .send()
        .await
        .unwrap();

    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/InRelease",
            repo.s3_prefix, repo.distribution
        ))
        .body(axum::body::Bytes::from_owner(payload.clearsigned).into())
        .send()
        .await
        .unwrap();

    // Get packages marked for removal.
    let removed_packages = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.id,
            debian_repository_package.filename
        FROM
            debian_repository_release
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE
            debian_repository_release.id = $1
            AND debian_repository_package.staging_status = 'remove'
        "#,
        release_id as i64
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    // Delete removed package files from S3 and delete from database.
    for removed in removed_packages {
        // Delete the package file from S3
        state
            .s3
            .delete_object()
            .bucket(&repo.s3_bucket)
            .key(format!("{}/{}", repo.s3_prefix, removed.filename))
            .send()
            .await
            .unwrap();
            
        // Delete the package from the database
        sqlx::query!(
            r#"
            DELETE FROM debian_repository_package
            WHERE id = $1
            "#,
            removed.id
        )
        .execute(&mut *tx)
        .await
        .unwrap();
    }

    // Update staging statuses for all packages.
    sqlx::query!(
        r#"
            UPDATE debian_repository_package
            SET staging_status = NULL
            WHERE id IN (
                SELECT debian_repository_package.id
                FROM
                    debian_repository_release
                    JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
                    JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
                WHERE
                    debian_repository_release.id = $1
                    AND debian_repository_package.staging_status = 'add'
            )
        "#,
        release_id as i64
    )
    .execute(&mut *tx)
    .await
    .unwrap();

    tx.commit().await.unwrap();

    // TODO: Clean up staging.

    Ok(())
}
