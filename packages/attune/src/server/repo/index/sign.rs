use aws_sdk_s3::types::ChecksumAlgorithm;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use base64::Engine as _;
use lazy_regex::lazy_regex;
use md5::{Digest as _, Md5};
use pgp::composed::{
    CleartextSignedMessage, Deserializable as _, SignedPublicKey, StandaloneSignature,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use time::OffsetDateTime;
use tracing::{debug, instrument};

use crate::{
    api::{ErrorResponse, TenantID},
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            index::{
                PackageChange, PackageChangeAction, PackageChangeResult,
                generate_release_file_with_change,
            },
        },
    },
};

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexRequest {
    pub change: PackageChange,
    pub release_ts: OffsetDateTime,
    pub clearsigned: String,
    pub detachsigned: String,
    pub public_key_cert: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SignIndexResponse {}

#[axum::debug_handler]
#[instrument(skip(state, req))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(repo_name): Path<String>,
    Json(req): Json<SignIndexRequest>,
) -> Result<Json<SignIndexResponse>, ErrorResponse> {
    debug!(?req, "signing index");

    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;
    if repo_name != req.change.repository {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "REPOSITORY_MISMATCH".to_string(),
            "repository name in path does not match repository name in request".to_string(),
        ));
    }

    if !lazy_regex!(r"^[a-zA-Z0-9_-]+$").is_match(&req.change.component) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            String::from("INVALID_COMPONENT_NAME"),
            String::from(
                "component name must contain only letters, numbers, underscores, and hyphens",
            ),
        ));
    }

    // Start a Serializable database transaction.
    let mut tx = state.db.begin().await.map_err(ErrorResponse::from)?;
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(ErrorResponse::from)?;

    // Load the repository. If it does not exist, return an error.
    let repo = sqlx::query_as!(
        Repository,
        r#"
        SELECT s3_bucket, s3_prefix
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        repo_name
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ErrorResponse::from)?
    .ok_or(ErrorResponse::not_found("repository"))?;

    // Apply the change to the database.
    let (result, previous_by_hash_indexes) = apply_change_to_db(&mut tx, &tenant_id, &req).await?;

    // Commit the transaction. At this point, the transaction may abort because
    // of a concurrent index change. This should trigger the client to retry.
    //
    // Technically we should probably check for specific error codes,
    // but the overwhelmingly most likely cause of an error here is a concurrent
    // change so for now we just assume all errors are due to this.
    //
    // We've added logging here so that we can see the actual error code
    // and special case it in the future.
    tx.commit().await.map_err(ErrorResponse::from)?;

    // Save the new index state to S3. This must occur after the transaction
    // commits so that we are sure that we are not incorrectly overwriting a
    // concurrent index update.
    //
    // Note that there is a small risk of a handler crash _after_ the
    // transaction commits and _before_ the index update in S3 completes. This
    // will leave the repository in an inconsistent state, but can be fixed by
    // redoing the S3 upload from database state.
    //
    // There's also a small risk that two changes A and B will commit in order
    // `A -> B`, but will upload indexes to S3 in order `B -> A`, which will
    // cause the older index to overwrite the newer one. This is also very
    // unlikely, but there is no good mitigation here besides a cron job. Note
    // that any _subsequent_ upload will still upload the correct indexes,
    // because the _database_ state is transactionally consistent.
    apply_change_to_s3(&state.s3, &repo, &req, &result, previous_by_hash_indexes).await;

    Ok(Json(SignIndexResponse {}))
}

async fn apply_change_to_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
) -> Result<(PackageChangeResult, Option<PreviousByHashIndexes>), ErrorResponse> {
    // Verify the request cleartext signature.
    let (public_key, _headers) = SignedPublicKey::from_string(&req.public_key_cert)
        .expect("could not parse public key certificate");
    debug!(?public_key, "public key");
    if let Err(e) = public_key.verify() {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "PUBLIC_KEY_VERIFICATION_FAILED".to_string(),
            format!("could not verify public key: {e}"),
        ));
    }
    let (clearsigned, _headers) = CleartextSignedMessage::from_string(&req.clearsigned)
        .expect("could not parse clearsigned index");
    debug!(clearsigned = ?clearsigned.text(), "clearsigned index");
    if let Err(e) = clearsigned.verify(&public_key) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "CLEARSIGN_VERIFICATION_FAILED".to_string(),
            format!("could not verify clearsigned index: {e}"),
        ));
    }

    // Replay the diff onto the current state of the index. Since index
    // generation is deterministic, this should yield the same index that was
    // signed locally.
    let result =
        generate_release_file_with_change(tx, tenant_id, &req.change, req.release_ts).await?;
    debug!(?result, "replayed index");

    // Compare the replayed index with the signed index.
    // If the signatures match, this validates that the index signed by the client
    // is the same as the one we replayed.
    let (detachsigned, _headers) = StandaloneSignature::from_string(&req.detachsigned)
        .expect("could not parse detached signature");
    debug!(index = ?result.release_file.contents, ?detachsigned, "detachsigned index");
    if let Err(e) = detachsigned.verify(&public_key, result.release_file.contents.as_bytes()) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "DETACHED_SIGNATURE_VERIFICATION_FAILED".to_string(),
            format!(
                "could not verify detached signature (index content mismatch or signature invalid): {e}"
            ),
        ));
    }

    // Save the new state to the database.
    let previous_by_hash_indexes = match req.change.action {
        PackageChangeAction::Add { .. } => add_package_to_db(tx, tenant_id, req, &result).await?,
        PackageChangeAction::Remove {
            ref name,
            ref version,
            ref architecture,
        } => Some(
            remove_package_from_db(tx, tenant_id, req, &result, name, version, architecture)
                .await?,
        ),
    };

    Ok((result, previous_by_hash_indexes))
}

#[derive(Debug)]
struct PreviousByHashIndexes {
    md5sum: String,
    sha1sum: String,
    sha256sum: String,
}

async fn add_package_to_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
    update: &PackageChangeResult,
) -> Result<Option<PreviousByHashIndexes>, ErrorResponse> {
    // First, we update-or-create the Release. Remember, it's possible that no
    // package has ever been added to this distribution, so the Release may not
    // exist.
    let release_id = match sqlx::query!(r#"
        SELECT
            debian_repository_release.id,
            debian_repository_release.description,
            debian_repository_release.origin,
            debian_repository_release.label,
            debian_repository_release.version,
            debian_repository_release.suite,
            debian_repository_release.codename,
            debian_repository_release.contents,
            debian_repository_release.clearsigned,
            debian_repository_release.detached
        FROM
            debian_repository
            JOIN debian_repository_release ON debian_repository.id = debian_repository_release.repository_id
        WHERE
            debian_repository.tenant_id = $1
            AND debian_repository.name = $2
            AND debian_repository_release.distribution = $3
        LIMIT 1
        "#,
        tenant_id.0,
        req.change.repository,
        req.change.distribution,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(ErrorResponse::from)? {
        Some(release) => {
            // If the release already exists, check whether any fields need to
            // be updated. If so, update them.
            if release.description != update.release_file.meta.description ||
                release.origin != update.release_file.meta.origin ||
                release.label != update.release_file.meta.label ||
                release.version != update.release_file.meta.version ||
                release.suite != update.release_file.meta.suite ||
                release.codename != update.release_file.meta.codename ||
                release.contents != update.release_file.contents ||
                release.clearsigned.is_none() ||
                release.clearsigned.is_some_and(|clearsigned| clearsigned != req.clearsigned) ||
                release.detached.is_none() ||
                release.detached.is_some_and(|detached| detached != req.detachsigned) {
                sqlx::query!(
                    r#"
                    UPDATE
                        debian_repository_release
                    SET
                        description = $2,
                        origin = $3,
                        label = $4,
                        version = $5,
                        suite = $6,
                        codename = $7,
                        contents = $8,
                        clearsigned = $9,
                        detached = $10,
                        updated_at = NOW()
                    WHERE
                        id = $1
                    "#,
                    release.id,
                    update.release_file.meta.description,
                    update.release_file.meta.origin,
                    update.release_file.meta.label,
                    update.release_file.meta.version,
                    update.release_file.meta.suite,
                    update.release_file.meta.codename,
                    update.release_file.contents,
                    req.clearsigned,
                    req.detachsigned,
                )
                .execute(&mut **tx)
                .await
                .map_err(ErrorResponse::from)?;
            }
            release.id
        },
        None => {
            // If the release doesn't exist, create it with default values.
            let release = sqlx::query!(
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
                    clearsigned,
                    detached,
                    created_at,
                    updated_at
                )
                SELECT
                    debian_repository.id,
                    $3,
                    $4,
                    $5,
                    $6,
                    $7,
                    $8,
                    $9,
                    $10,
                    $11,
                    $12,
                    NOW(),
                    NOW()
                FROM debian_repository
                WHERE
                    debian_repository.tenant_id = $1
                    AND debian_repository.name = $2
                RETURNING id
                "#,
                tenant_id.0,
                req.change.repository,
                req.change.distribution,
                update.release_file.meta.description,
                update.release_file.meta.origin,
                update.release_file.meta.label,
                update.release_file.meta.version,
                update.release_file.meta.suite,
                update.release_file.meta.codename,
                update.release_file.contents,
                req.clearsigned,
                req.detachsigned,
            )
            .fetch_one(&mut **tx)
            .await
            .map_err(ErrorResponse::from)?;
            release.id
        }
    };

    // Then, we find-or-create the Component.
    let component_id = match sqlx::query!(
        r#"
        SELECT id
        FROM debian_repository_component
        WHERE release_id = $1 AND name = $2
        LIMIT 1
        "#,
        release_id,
        req.change.component,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?
    {
        Some(component) => component.id,
        None => {
            sqlx::query!(
                r#"
                INSERT INTO debian_repository_component (
                    release_id,
                    name,
                    created_at,
                    updated_at
                )
                VALUES (
                    $1,
                    $2,
                    NOW(),
                    NOW()
                )
                RETURNING id
                "#,
                release_id,
                req.change.component,
            )
            .fetch_one(&mut **tx)
            .await
            .map_err(ErrorResponse::from)?
            .id
        }
    };

    // Then, we update-or-create the Packages index of the changed package.
    let previous_by_hash_indexes = match sqlx::query!(
        r#"
        SELECT id, md5sum, sha1sum, sha256sum
        FROM debian_repository_index_packages
        WHERE
            component_id = $1
            AND architecture = $2::debian_repository_architecture
            AND compression IS NULL
        LIMIT 1
        "#,
        component_id,
        update.changed_packages_index.meta.architecture as _,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?
    {
        Some(index) => {
            // Before we do an update, we need to capture the hashes of the
            // previous Packages index since its by-hash files need to be
            // deleted after the update.
            let previous_by_hash_indexes = PreviousByHashIndexes {
                md5sum: index.md5sum,
                sha1sum: index.sha1sum,
                sha256sum: index.sha256sum,
            };

            // No need to check whether an update is needed - we know already
            // that the index has changed because a package was added into it.
            sqlx::query!(
                r#"
                UPDATE debian_repository_index_packages
                SET
                    contents = $2,
                    size = $3,
                    md5sum = $4,
                    sha1sum = $5,
                    sha256sum = $6,
                    updated_at = NOW()
                WHERE id = $1
                "#,
                index.id,
                update.changed_packages_index.contents.as_bytes(),
                update.changed_packages_index.meta.size,
                update.changed_packages_index.meta.md5sum,
                update.changed_packages_index.meta.sha1sum,
                update.changed_packages_index.meta.sha256sum,
            )
            .execute(&mut **tx)
            .await
            .map_err(ErrorResponse::from)?;
            Some(previous_by_hash_indexes)
        }
        None => {
            // Otherwise, create the index.
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
                    created_at,
                    updated_at
                )
                VALUES (
                    $1,
                    $2::debian_repository_architecture,
                    NULL,
                    $3,
                    $4,
                    $5,
                    $6,
                    $7,
                    NOW(),
                    NOW()
                )
                "#,
                component_id,
                update.changed_packages_index.meta.architecture as _,
                // compression = NULL,
                update.changed_packages_index.meta.size,
                update.changed_packages_index.contents.as_bytes(),
                update.changed_packages_index.meta.md5sum,
                update.changed_packages_index.meta.sha1sum,
                update.changed_packages_index.meta.sha256sum,
            )
            .execute(&mut **tx)
            .await
            .map_err(ErrorResponse::from)?;
            None
        }
    };

    // Lastly, we create the component-package.
    //
    // This record should not previously exist, but we use ON CONFLICT DO
    // NOTHING because we consider re-adding an identical package to be a no-op
    // rather than an error.
    sqlx::query!(
        r#"
        WITH package_cte AS (
            SELECT id
            FROM debian_repository_package
            WHERE
                tenant_id = $1
                AND sha256sum = $2
            LIMIT 1
        )
        INSERT INTO debian_repository_component_package (
            component_id,
            package_id,
            filename,
            created_at,
            updated_at
        )
        SELECT
            $3,
            package_cte.id,
            $4,
            NOW(),
            NOW()
        FROM package_cte
        ON CONFLICT DO NOTHING
        "#,
        tenant_id.0,
        update.changed_package.package.sha256sum,
        component_id,
        update.changed_package.filename,
    )
    .execute(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?;

    Ok(previous_by_hash_indexes)
}

async fn remove_package_from_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
    update: &PackageChangeResult,
    package: &str,
    version: &str,
    architecture: &str,
) -> Result<PreviousByHashIndexes, ErrorResponse> {
    // Load the component-package, which should be there if the package exists.
    let component_package = sqlx::query!(
        r#"
        SELECT
            debian_repository_component_package.package_id,
            debian_repository_component_package.component_id
        FROM
            debian_repository
            JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_component_package ON debian_repository_component_package.component_id = debian_repository_component.id
            JOIN debian_repository_package ON debian_repository_package.id = debian_repository_component_package.package_id
        WHERE
            debian_repository.tenant_id = $1
            AND debian_repository.name = $2
            AND debian_repository_release.distribution = $3
            AND debian_repository_component.name = $4
            AND debian_repository_package.package = $5
            AND debian_repository_package.version = $6
            AND debian_repository_package.architecture = $7::debian_repository_architecture
        LIMIT 1
        "#,
        tenant_id.0,
        req.change.repository,
        req.change.distribution,
        req.change.component,
        package,
        version,
        architecture as _,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?;

    // Delete the component-package.
    sqlx::query!(
        r#"
        DELETE FROM debian_repository_component_package
        WHERE
            component_id = $1
            AND package_id = $2
        "#,
        component_package.component_id,
        component_package.package_id,
    )
    .execute(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?;

    // Load the current state of the changed Packages index. We need to record
    // its hashes so that we can delete the by-hash files after we update this
    // index.
    let previous_by_hash_indexes = sqlx::query!(
        r#"
        SELECT
            md5sum,
            sha1sum,
            sha256sum
        FROM debian_repository_index_packages
        WHERE
            component_id = $1
            AND architecture = $2::debian_repository_architecture
            AND compression IS NULL
        LIMIT 1
        "#,
        component_package.component_id,
        architecture as _,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?;
    let previous_by_hash_indexes = PreviousByHashIndexes {
        md5sum: previous_by_hash_indexes.md5sum,
        sha1sum: previous_by_hash_indexes.sha1sum,
        sha256sum: previous_by_hash_indexes.sha256sum,
    };

    // Update the Packages index, or delete if it's orphaned.
    if update.changed_packages_index.contents.is_empty() {
        sqlx::query!(
            r#"
            DELETE FROM debian_repository_index_packages
            WHERE
                component_id = $1
                AND architecture = $2::debian_repository_architecture
        "#,
            component_package.component_id,
            architecture as _,
        )
        .execute(&mut **tx)
        .await
        .map_err(ErrorResponse::from)?;
    } else {
        sqlx::query!(
            r#"
            UPDATE debian_repository_index_packages
            SET
                contents = $1,
                size = $2,
                md5sum = $3,
                sha1sum = $4,
                sha256sum = $5,
                updated_at = NOW()
            WHERE
                component_id = $6
                AND architecture = $7::debian_repository_architecture
                AND compression IS NULL
            "#,
            update.changed_packages_index.contents.as_bytes(),
            update.changed_packages_index.meta.size,
            update.changed_packages_index.meta.md5sum,
            update.changed_packages_index.meta.sha1sum,
            update.changed_packages_index.meta.sha256sum,
            component_package.component_id,
            architecture as _,
        )
        .execute(&mut **tx)
        .await
        .map_err(ErrorResponse::from)?;
    }

    // Delete the Component if it's orphaned.
    let remaining_component_packages = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!: i64"
        FROM debian_repository_component_package
        WHERE component_id = $1
        "#,
        component_package.component_id,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ErrorResponse::from)?;
    if remaining_component_packages.count == 0 {
        sqlx::query!(
            r#"
            DELETE FROM debian_repository_component
            WHERE id = $1
        "#,
            component_package.component_id,
        )
        .execute(&mut **tx)
        .await
        .map_err(ErrorResponse::from)?;
    }

    // We do not delete the Release even if it's orphaned, because
    // clients may still be pointing to the release file, and we don't
    // want them to be broken. The error they should get from APT is
    // "package missing", rather than "repository not found".

    Ok(previous_by_hash_indexes)
}

struct Repository {
    s3_bucket: String,
    s3_prefix: String,
}

async fn apply_change_to_s3(
    s3: &aws_sdk_s3::Client,
    repo: &Repository,
    req: &SignIndexRequest,
    result: &PackageChangeResult,
    previous_by_hash_indexes: Option<PreviousByHashIndexes>,
) {
    // Copy the package from its canonical storage location into the repository
    // pool.
    match req.change.action {
        PackageChangeAction::Add { .. } => {
            let source_key = format!(
                "{}/packages/{}",
                result.changed_package.package.s3_bucket, result.changed_package.package.sha256sum,
            );
            let destination_key = format!("{}/{}", repo.s3_prefix, result.changed_package.filename);
            debug!(?source_key, ?destination_key, "copy package to pool");
            s3.copy_object()
                .bucket(&repo.s3_bucket)
                .key(destination_key)
                .copy_source(source_key)
                .send()
                .await
                .unwrap();
        }
        PackageChangeAction::Remove { .. } => {
            // Delete the pool file from S3 if it's fully orphaned.
            let key = format!("{}/{}", repo.s3_prefix, result.changed_package.filename);
            debug!(?key, "delete pool file from S3");
            if result.orphaned_pool_filename {
                s3.delete_object()
                    .bucket(&repo.s3_bucket)
                    .key(key)
                    .send()
                    .await
                    .unwrap();
            }
        }
    }

    // Upload the updated package index files to standard path and all by-hash
    // paths concurrently.
    //
    // Index modifications are split on both sides of the release file upload:
    // - Before release files are uploaded, we upload new index contents.
    // - After release files are uploaded, we delete old index contents.
    //
    // The intention here is that the current release file _always points to
    // valid files_.
    let by_hash_prefix = format!(
        "{}/dists/{}/{}/binary-{}/by-hash",
        repo.s3_prefix,
        req.change.distribution,
        result.changed_packages_index.meta.component,
        result.changed_packages_index.meta.architecture
    );
    if !result.changed_packages_index.contents.is_empty() {
        let uploads = [
            format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix,
                req.change.distribution,
                result.changed_packages_index.meta.component,
                result.changed_packages_index.meta.architecture
            ),
            format!(
                "{}/SHA256/{}",
                by_hash_prefix, result.changed_packages_index.meta.sha256sum
            ),
            format!(
                "{}/SHA1/{}",
                by_hash_prefix, result.changed_packages_index.meta.sha1sum
            ),
            format!(
                "{}/MD5Sum/{}",
                by_hash_prefix, result.changed_packages_index.meta.md5sum
            ),
        ]
        .into_iter()
        .map(|key: String| {
            let bucket = &repo.s3_bucket;
            let contents = &result.changed_packages_index.contents;
            let sha256sum = &result.changed_packages_index.meta.sha256sum;

            async move {
                debug!(?key, content = %contents, "uploading index file");
                s3.put_object()
                    .bucket(bucket)
                    .key(key)
                    .content_md5(
                        base64::engine::general_purpose::STANDARD
                            .encode(Md5::digest(contents.as_bytes())),
                    )
                    .checksum_algorithm(ChecksumAlgorithm::Sha256)
                    .checksum_sha256(
                        base64::engine::general_purpose::STANDARD
                            .encode(hex::decode(sha256sum).unwrap()),
                    )
                    .body(contents.as_bytes().to_vec().into())
                    .send()
                    .await
            }
        });
        for upload in futures_util::future::join_all(uploads).await {
            upload.unwrap();
        }
    }

    // Upload the updated Release files. This must happen after package uploads
    // and index uploads so that all files are in place for Acquire-By-Hash.
    let uploads = [
        (
            format!(
                "{}/dists/{}/InRelease",
                repo.s3_prefix, req.change.distribution
            ),
            req.clearsigned.as_bytes().to_vec(),
        ),
        (
            format!(
                "{}/dists/{}/Release",
                repo.s3_prefix, req.change.distribution
            ),
            result.release_file.contents.as_bytes().to_vec(),
        ),
        (
            format!(
                "{}/dists/{}/Release.gpg",
                repo.s3_prefix, req.change.distribution
            ),
            req.detachsigned.as_bytes().to_vec(),
        ),
    ]
    .into_iter()
    .map(|(key, content)| {
        debug!(?key, content = %String::from_utf8_lossy(&content), "uploading release file");
        s3.put_object()
            .bucket(&repo.s3_bucket)
            .key(key)
            .content_md5(base64::engine::general_purpose::STANDARD.encode(Md5::digest(&content)))
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .checksum_sha256(
                base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&content)),
            )
            .body(content.into())
            .send()
    });
    for upload in futures_util::future::join_all(uploads).await {
        upload.unwrap();
    }

    // Now we can do deletions: the release files are uploaded and are no longer
    // pointing at the by-hash Packages indexes that we're about to delete.
    let deletions = match previous_by_hash_indexes {
        None => Vec::new(),
        Some(PreviousByHashIndexes {
            md5sum,
            sha1sum,
            sha256sum,
        }) => [
            (md5sum, &result.changed_packages_index.meta.md5sum, "MD5Sum"),
            (sha1sum, &result.changed_packages_index.meta.sha1sum, "SHA1"),
            (
                sha256sum,
                &result.changed_packages_index.meta.sha256sum,
                "SHA256",
            ),
        ]
        .into_iter()
        // This step is needed because the old hash might equal the new hash!
        // This can occur if you upload a package that was already in the index,
        // in which case adding the package to the index is a no-op. In that
        // case, we don't want to delete the "old" (but actually still
        // up-to-date) index.
        .filter(|(old_hash, new_hash, _)| &old_hash != new_hash)
        .map(|(old_hash, _, hash_type)| format!("{by_hash_prefix}/{hash_type}/{old_hash}"))
        .collect::<Vec<_>>(),
    };
    debug!(?deletions, "deletions");

    // S3 only allows up to 1000 objects per delete request, but we're dealing
    // with low ones of keys.
    let keys = deletions
        .into_iter()
        .map(|key| {
            aws_sdk_s3::types::ObjectIdentifier::builder()
                .key(key)
                .build()
                .unwrap()
        })
        .collect::<Vec<_>>();
    if !keys.is_empty() {
        let delete = aws_sdk_s3::types::Delete::builder()
            .set_objects(Some(keys))
            .build()
            .unwrap();
        let deletion = s3
            .delete_objects()
            .bucket(&repo.s3_bucket)
            .delete(delete)
            .send()
            .await;
        if let Err(err) = deletion {
            tracing::error!("Failed to delete objects: {err:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::iter::once;

    use axum_test::multipart::{MultipartForm, Part};
    use gpgme::ExportMode;
    use tracing::info;

    use super::*;
    use crate::{
        server::{
            pkg::upload::PackageUploadResponse,
            repo::{
                index::generate::{GenerateIndexRequest, GenerateIndexResponse},
                sync::check::CheckConsistencyResponse,
            },
        },
        testing::{AttuneTestServer, AttuneTestServerConfig, fixtures, gpg_key_id},
    };

    // TODO: Replace with thge new centralized gpg_sign function.
    async fn sign_index(index: &str) -> (String, String, String) {
        let (key_id, mut gpg, _dir) = gpg_key_id().await.expect("failed to create GPG key");
        let key = gpg
            .find_secret_keys(vec![key_id])
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        gpg.add_signer(&key).unwrap();

        let mut clearsigned = Vec::new();
        gpg.sign_clear(index.as_bytes(), &mut clearsigned)
            .expect("could not clearsign index");
        let clearsigned =
            String::from_utf8(clearsigned).expect("clearsigned index contained invalid characters");
        debug!(?index, ?clearsigned, "clearsigned index");
        let mut detachsigned = Vec::new();
        gpg.sign_detached(index.as_bytes(), &mut detachsigned)
            .expect("could not detach sign index");
        let detachsigned = String::from_utf8(detachsigned)
            .expect("detachsigned index contained invalid characters");
        debug!(?index, ?detachsigned, "detachsigned index");

        let mut public_key_cert = Vec::new();
        gpg.export_keys(once(&key), ExportMode::empty(), &mut public_key_cert)
            .expect("could not export key");
        let public_key_cert = String::from_utf8(public_key_cert)
            .expect("public key cert contained invalid characters");
        debug!(?public_key_cert, "public key cert");

        (clearsigned, detachsigned, public_key_cert)
    }

    #[sqlx::test(migrator = "crate::testing::MIGRATOR")]
    #[test_log::test]
    async fn resync_mitigates_partial_upload(pool: sqlx::PgPool) {
        let server = AttuneTestServer::new(AttuneTestServerConfig {
            db: pool,
            s3_bucket_name: None,
            http_api_token: None,
        })
        .await;
        const REPO_NAME: &str = "resync_mitigates_partial_upload";
        let (tenant_id, api_token) = server.create_test_tenant(REPO_NAME).await;

        // Set up an empty repository.
        let s3_prefix = server.create_repository(tenant_id, REPO_NAME).await;
        info!(name = ?REPO_NAME, ?s3_prefix, "created repository");

        // Upload a package.
        let package_file = fixtures::TEST_PACKAGE_AMD64;
        let upload = MultipartForm::new().add_part("file", Part::bytes(package_file.to_vec()));

        let res = server
            .http
            .post("/api/v0/packages")
            .add_header("authorization", format!("Bearer {api_token}"))
            .multipart(upload)
            .await;
        assert!(
            res.status_code().is_success(),
            "Package upload failed with status: {}",
            res.status_code()
        );
        let res = res.json::<PackageUploadResponse>();
        let package_sha256sum = res.sha256sum;

        // Generate an index to sign.
        let req = GenerateIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),

                action: PackageChangeAction::Add {
                    package_sha256sum: package_sha256sum.clone(),
                },
            },
        };

        let res = server
            .http
            .get(&format!("/api/v0/repositories/{REPO_NAME}/index"))
            .add_header("authorization", format!("Bearer {api_token}"))
            .json(&req)
            .await;
        assert!(
            res.status_code().is_success(),
            "Index generation failed with status: {}",
            res.status_code()
        );
        let res = res.json::<GenerateIndexResponse>();
        let release_ts = res.release_ts;
        let index = res.release;

        // Sign the index. We use a temporary homedir to create an ephemeral key
        // and sign the index with it.
        let (clearsigned, detachsigned, public_key_cert) = sign_index(&index).await;

        // At this point, we begin running the `sign` handler piecemeal to
        // simulate partial failure.

        // "Submit" the index signature, running the handler manually. Complete
        // the database transaction.
        //
        // NOTE: This skips request validation in the handler upstream of the
        // database transaction! This test will not reflect updates in
        // validation, and should be constructed to pass validation.
        let req = SignIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),
                action: PackageChangeAction::Add { package_sha256sum },
            },
            clearsigned,
            detachsigned,
            public_key_cert,
            release_ts,
        };
        let mut tx = server.db.begin().await.unwrap();
        let (result, _) = apply_change_to_db(&mut tx, &tenant_id, &req).await.unwrap();
        tx.commit().await.unwrap();

        // Partially upload the index changes. In this case, we upload the
        // package and index but fail to upload all release files.
        //
        // TODO: Is there a way to automatically permute possible failure
        // points?

        // Copy the package from its canonical storage location into the
        // repository pool.
        server
            .s3
            .copy_object()
            .bucket(&server.s3_bucket_name)
            .key(format!("{}/{}", s3_prefix, result.changed_package.filename))
            .copy_source(format!(
                "{}/packages/{}",
                server.s3_bucket_name, result.changed_package.package.sha256sum,
            ))
            .send()
            .await
            .unwrap();

        // Upload the updated Packages index file.
        server
            .s3
            .put_object()
            .bucket(&server.s3_bucket_name)
            .key(format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                s3_prefix,
                req.change.distribution,
                result.changed_packages_index.meta.component,
                result.changed_packages_index.meta.architecture
            ))
            .content_md5(
                base64::engine::general_purpose::STANDARD.encode(Md5::digest(
                    result.changed_packages_index.contents.as_bytes(),
                )),
            )
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .checksum_sha256(
                base64::engine::general_purpose::STANDARD
                    .encode(hex::decode(&result.changed_packages_index.meta.sha256sum).unwrap()),
            )
            .body(
                result
                    .changed_packages_index
                    .contents
                    .as_bytes()
                    .to_vec()
                    .into(),
            )
            .send()
            .await
            .unwrap();

        // Partially update the release files.
        server
            .s3
            .put_object()
            .bucket(&server.s3_bucket_name)
            .key(format!(
                "{}/dists/{}/Release",
                s3_prefix, req.change.distribution
            ))
            .content_md5(
                base64::engine::general_purpose::STANDARD
                    .encode(Md5::digest(result.release_file.contents.as_bytes())),
            )
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .checksum_sha256(
                base64::engine::general_purpose::STANDARD
                    .encode(Sha256::digest(result.release_file.contents.as_bytes())),
            )
            .body(result.release_file.contents.as_bytes().to_vec().into())
            .send()
            .await
            .unwrap();

        // Check that we can detect the desynchronization.
        let res = server
            .http
            .get(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync check failed with status: {}",
            res.status_code()
        );
        let status = res.json::<CheckConsistencyResponse>().status;
        debug!(?status, "sync check result");
        assert!(!status.release, "Release file is inconsistent");
        assert!(
            status.release_clearsigned,
            "InRelease file inconsistency was not detected"
        );
        assert!(
            status.release_detachsigned,
            "Release.gpg file inconsistency was not detected"
        );
        assert_eq!(
            status.packages,
            vec![] as Vec<String>,
            "Packages are inconsistent"
        );
        let actual_inconsistent_packages_indexes = {
            let mut pis = status.packages_indexes.clone();
            pis.sort();
            pis
        };
        assert_eq!(
            actual_inconsistent_packages_indexes,
            vec![
                "dists/stable/main/binary-amd64/by-hash/MD5Sum/f80941151667622221b97e18dc854ecd",
                "dists/stable/main/binary-amd64/by-hash/SHA1/cc7bc34cfd50b7418c673477515828da8bbdeb60",
                "dists/stable/main/binary-amd64/by-hash/SHA256/bc5815deb20e1ea8a750a3e83de009f87d492ae13518198a3f59373145912cad",
            ],
            "Packages indexes are inconsistent"
        );

        // Resync the repository.
        let res = server
            .http
            .post(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync failed with status: {}",
            res.status_code()
        );

        // Check that the repository is synchronized.
        let res = server
            .http
            .get(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync check failed with status: {}",
            res.status_code()
        );
        let status = res.json::<CheckConsistencyResponse>().status;
        debug!(?status, "sync check result");
        assert!(!status.release, "Release file is inconsistent");
        assert!(
            !status.release_clearsigned,
            "InRelease file is inconsistent"
        );
        assert!(
            !status.release_detachsigned,
            "Release.gpg file is inconsistent"
        );
        assert!(status.packages.is_empty(), "Packages are inconsistent");
        assert!(
            status.packages_indexes.is_empty(),
            "Packages indexes are inconsistent"
        );
    }

    #[sqlx::test(migrator = "crate::testing::MIGRATOR")]
    async fn resync_mitigates_out_of_order_upload(pool: sqlx::PgPool) {
        let server = AttuneTestServer::new(AttuneTestServerConfig {
            db: pool,
            s3_bucket_name: None,
            http_api_token: None,
        })
        .await;
        const REPO_NAME: &str = "resync_mitigates_out_of_order_upload";
        let (tenant_id, api_token) = server.create_test_tenant(REPO_NAME).await;

        // Set up an empty repository.
        let s3_prefix = server.create_repository(tenant_id, REPO_NAME).await;

        // Upload packages.
        let package_file_a = fixtures::TEST_PACKAGE_AMD64;
        let upload = MultipartForm::new().add_part("file", Part::bytes(package_file_a.to_vec()));
        let res = server
            .http
            .post("/api/v0/packages")
            .add_header("authorization", format!("Bearer {api_token}"))
            .multipart(upload)
            .await;
        assert!(
            res.status_code().is_success(),
            "Package upload failed with status: {}",
            res.status_code()
        );
        let res = res.json::<PackageUploadResponse>();
        let package_a_sha256sum = res.sha256sum;

        let package_file_b = fixtures::TEST_PACKAGE_ARM64;
        let upload = MultipartForm::new().add_part("file", Part::bytes(package_file_b.to_vec()));
        let res = server
            .http
            .post("/api/v0/packages")
            .add_header("authorization", format!("Bearer {api_token}"))
            .multipart(upload)
            .await;
        assert!(
            res.status_code().is_success(),
            "Package upload failed with status: {}",
            res.status_code()
        );
        let res = res.json::<PackageUploadResponse>();
        let package_b_sha256sum = res.sha256sum;

        // At this point, we begin running the sign handler piecemeal. Here, we
        // first run the database transactions in sequence, and then run the
        // repository uploads in a different sequence, to simulate a rare race
        // condition where another an earlier handler invocation updates S3
        // after a later handler invocation.

        // Add package 1 to the database.
        let req = GenerateIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),

                action: PackageChangeAction::Add {
                    package_sha256sum: package_a_sha256sum.clone(),
                },
            },
        };
        let res = server
            .http
            .get(&format!("/api/v0/repositories/{REPO_NAME}/index"))
            .add_header("authorization", format!("Bearer {api_token}"))
            .json(&req)
            .await;
        assert!(
            res.status_code().is_success(),
            "Index generation failed with status: {}",
            res.status_code()
        );
        let res = res.json::<GenerateIndexResponse>();
        let release_ts = res.release_ts;
        let index = res.release;
        let (clearsigned, detachsigned, public_key_cert) = sign_index(&index).await;
        let req_a = SignIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),
                action: PackageChangeAction::Add {
                    package_sha256sum: package_a_sha256sum,
                },
            },
            clearsigned,
            detachsigned,
            public_key_cert,
            release_ts,
        };
        let mut tx = server.db.begin().await.unwrap();
        let (result_a, previous_by_hash_indexes_a) =
            apply_change_to_db(&mut tx, &tenant_id, &req_a)
                .await
                .unwrap();
        debug!(?result_a, "applied change to database");
        tx.commit().await.unwrap();

        // Add package 2 to the database.
        let req = GenerateIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),

                action: PackageChangeAction::Add {
                    package_sha256sum: package_b_sha256sum.clone(),
                },
            },
        };
        let res = server
            .http
            .get(&format!("/api/v0/repositories/{REPO_NAME}/index"))
            .add_header("authorization", format!("Bearer {api_token}"))
            .json(&req)
            .await;
        assert!(
            res.status_code().is_success(),
            "Index generation failed with status: {}",
            res.status_code()
        );
        let res = res.json::<GenerateIndexResponse>();
        let release_ts = res.release_ts;
        let index = res.release;
        let (clearsigned, detachsigned, public_key_cert) = sign_index(&index).await;
        let req_b = SignIndexRequest {
            change: PackageChange {
                repository: String::from(REPO_NAME),
                distribution: String::from("stable"),
                component: String::from("main"),
                action: PackageChangeAction::Add {
                    package_sha256sum: package_b_sha256sum,
                },
            },
            clearsigned,
            detachsigned,
            public_key_cert,
            release_ts,
        };
        let mut tx = server.db.begin().await.unwrap();
        let (result_b, previous_by_hash_indexes_b) =
            apply_change_to_db(&mut tx, &tenant_id, &req_b)
                .await
                .unwrap();
        debug!(?result_b, "applied change to database");
        tx.commit().await.unwrap();

        // Upload package 2 to the repository.
        apply_change_to_s3(
            &server.s3,
            &Repository {
                s3_bucket: server.s3_bucket_name.clone(),
                s3_prefix: s3_prefix.clone(),
            },
            &req_b,
            &result_b,
            previous_by_hash_indexes_b,
        )
        .await;

        // Upload package 1 to the repository.
        apply_change_to_s3(
            &server.s3,
            &Repository {
                s3_bucket: server.s3_bucket_name.clone(),
                s3_prefix: s3_prefix.clone(),
            },
            &req_a,
            &result_a,
            previous_by_hash_indexes_a,
        )
        .await;

        // Check that we can detect the desynchronization.
        let res = server
            .http
            .get(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync check failed with status: {}",
            res.status_code()
        );
        let status = res.json::<CheckConsistencyResponse>().status;
        debug!(?status, "sync check result");
        assert!(
            status.release,
            "Release file inconsistency was not detected"
        );
        assert!(
            status.release_clearsigned,
            "InRelease file inconsistency was not detected"
        );
        assert!(
            status.release_detachsigned,
            "Release.gpg file inconsistency was not detected"
        );
        assert_eq!(
            status.packages,
            vec![] as Vec<String>,
            "Packages are inconsistent"
        );
        // In _this particular case_, the package indexes should be the same,
        // since the packages we've uploaded are different architectures and
        // therefore will go into different indexes. This is not always the
        // case!
        //
        // TODO: Add property-based testing for a wide swath of scenarios?
        assert_eq!(
            status.packages_indexes,
            vec![] as Vec<String>,
            "Packages indexes are inconsistent"
        );

        // Resync the repository.
        let res = server
            .http
            .post(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync failed with status: {}",
            res.status_code()
        );

        // Check that the repository is synchronized.
        let res = server
            .http
            .get(&format!(
                "/api/v0/repositories/{REPO_NAME}/distributions/stable/sync"
            ))
            .add_header("authorization", format!("Bearer {api_token}"))
            .await;
        assert!(
            res.status_code().is_success(),
            "Sync check failed with status: {}",
            res.status_code()
        );
        let status = res.json::<CheckConsistencyResponse>().status;
        debug!(?status, "sync check result");
        assert!(!status.release, "Release file is inconsistent");
        assert!(
            !status.release_clearsigned,
            "InRelease file is inconsistent"
        );
        assert!(
            !status.release_detachsigned,
            "Release.gpg file is inconsistent"
        );
        assert!(status.packages.is_empty(), "Packages are inconsistent");
        assert!(
            status.packages_indexes.is_empty(),
            "Packages indexes are inconsistent"
        );
    }

    #[sqlx::test(migrator = "crate::testing::MIGRATOR")]
    async fn reject_invalid_component_names(pool: sqlx::PgPool) {
        let server = AttuneTestServer::new(AttuneTestServerConfig {
            db: pool,
            s3_bucket_name: None,
            http_api_token: None,
        })
        .await;
        const REPO_NAME: &str = "reject_invalid_component_names";
        let (tenant_id, api_token) = server.create_test_tenant(REPO_NAME).await;

        server.create_repository(tenant_id, REPO_NAME).await;

        let invalid_components = [
            "comp with spaces",
            "comp@special",
            "comp#hash",
            "comp.dot",
            "comp/slash",
            "",
        ];
        for invalid_component in invalid_components {
            let sign_request = SignIndexRequest {
                change: PackageChange {
                    repository: String::from(REPO_NAME),
                    distribution: String::from("stable"),
                    component: String::from(invalid_component),
                    action: PackageChangeAction::Add {
                        package_sha256sum: String::from("dummy-sha256sum"),
                    },
                },
                release_ts: OffsetDateTime::now_utc(),
                clearsigned: String::from("dummy-clearsigned"),
                detachsigned: String::from("dummy-detachsigned"),
                public_key_cert: String::from("dummy-public-key"),
            };

            let response = server
                .http
                .post(&format!("/api/v0/repositories/{REPO_NAME}/index"))
                .add_header("authorization", format!("Bearer {api_token}"))
                .json(&sign_request)
                .await;
            assert_eq!(
                response.status_code(),
                400,
                "Invalid component name should return 400"
            );

            let error: ErrorResponse = response.json();
            assert_eq!(error.error, "INVALID_COMPONENT_NAME");
            assert!(
                error
                    .message
                    .contains("must contain only letters, numbers, underscores, and hyphens")
            );
        }

        // Test valid component names (these should get further before failing due to
        // dummy data)
        let valid_components = [
            "main",
            "contrib",
            "non-free",
            "my_component",
            "comp123",
            "test-component",
        ];
        for valid_component in valid_components {
            let sign_request = SignIndexRequest {
                change: PackageChange {
                    repository: String::from(REPO_NAME),
                    distribution: String::from("stable"),
                    component: String::from(valid_component),
                    action: PackageChangeAction::Add {
                        package_sha256sum: String::from("dummy-sha256sum"),
                    },
                },
                release_ts: OffsetDateTime::now_utc(),
                clearsigned: String::from("dummy-clearsigned"),
                detachsigned: String::from("dummy-detachsigned"),
                public_key_cert: String::from("dummy-public-key"),
            };
            let response = server
                .http
                .post(&format!("/api/v0/repositories/{REPO_NAME}/index"))
                .add_header("authorization", format!("Bearer {api_token}"))
                .json(&sign_request)
                .await;

            // These should not fail with INVALID_COMPONENT_NAME (although they will fail
            // due to dummy data)
            let body = response.json::<ErrorResponse>();
            assert_ne!(
                body.error, "INVALID_COMPONENT_NAME",
                "Valid component '{valid_component}' should not be rejected with INVALID_COMPONENT_NAME",
            );
        }
    }
}
