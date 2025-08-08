use aws_sdk_s3::types::ChecksumAlgorithm;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use base64::Engine as _;
use itertools::Itertools;
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
                Package, PackageChange, PackageChangeAction, generate_release_file_with_change,
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

    // Verify the index signatures.
    let (public_key, _headers) = SignedPublicKey::from_string(&req.public_key_cert)
        .expect("could not parse public key certificate");
    tracing::debug!(?public_key, "public key");
    if let Err(e) = public_key.verify() {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "PUBLIC_KEY_VERIFICATION_FAILED".to_string(),
            format!("could not verify public key: {e}"),
        ));
    }
    let (clearsigned, _headers) = CleartextSignedMessage::from_string(&req.clearsigned)
        .expect("could not parse clearsigned index");
    tracing::debug!(clearsigned = ?clearsigned.text(), "clearsigned index");
    if let Err(e) = clearsigned.verify(&public_key) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "CLEARSIGN_VERIFICATION_FAILED".to_string(),
            format!("could not verify clearsigned index: {e}"),
        ));
    }

    // Start a Serializable database transaction.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Load the repository. If it does not exist, return an error.
    //
    // TODO: This query is redundant with `generate_release_file_with_change`.
    // Is there a better way to refactor this code?
    let repo = match sqlx::query!(
        r#"
        SELECT id, name, s3_bucket, s3_prefix
        FROM debian_repository
        WHERE tenant_id = $1 AND name = $2
        "#,
        tenant_id.0,
        repo_name
    )
    .fetch_optional(&mut *tx)
    .await
    .unwrap()
    {
        Some(repo) => repo,
        None => {
            return Err(ErrorResponse::new(
                StatusCode::NOT_FOUND,
                "REPOSITORY_NOT_FOUND".to_string(),
                "repository not found".to_string(),
            ));
        }
    };

    // Replay the diff onto the current state of the index. Since index
    // generation is deterministic, this should yield the same index that was
    // signed locally.
    let result =
        generate_release_file_with_change(&mut tx, &tenant_id, &req.change, req.release_ts).await?;
    debug!(?result, "replayed index");

    // Compare the replayed index with the signed index.
    // If the signatures match, this validates that the index signed by the client is the same as the one we replayed.
    let (detachsigned, _headers) = StandaloneSignature::from_string(&req.detachsigned)
        .expect("could not parse detached signature");
    tracing::debug!(index = ?result.release_file.contents, ?detachsigned, "detachsigned index");
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
    // The old hash values are for cleanup after uploading new content.
    let old_hashes = match req.change.action {
        PackageChangeAction::Add { .. } => {
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
                    debian_repository_release.detached,
                    debian_repository.s3_bucket,
                    debian_repository.s3_prefix
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
                repo_name,
                req.change.distribution,
            )
            .fetch_optional(&mut *tx)
            .await
            .unwrap() {
                Some(release) => {
                    // If the release already exists, check whether any fields need to
                    // be updated. If so, update them.
                    if release.description != result.release_file.meta.description ||
                        release.origin != result.release_file.meta.origin ||
                        release.label != result.release_file.meta.label ||
                        release.version != result.release_file.meta.version ||
                        release.suite != result.release_file.meta.suite ||
                        release.codename != result.release_file.meta.codename ||
                        release.contents != result.release_file.contents ||
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
                            result.release_file.meta.description,
                            result.release_file.meta.origin,
                            result.release_file.meta.label,
                            result.release_file.meta.version,
                            result.release_file.meta.suite,
                            result.release_file.meta.codename,
                            result.release_file.contents,
                            req.clearsigned,
                            req.detachsigned,
                        )
                        .execute(&mut *tx)
                        .await
                        .unwrap();
                    }
                    release.id
                },
                None => {
                    // If the release doesn't exist, create it with default values.
                    let repo = sqlx::query!(
                        r#"
                        SELECT id, s3_bucket, s3_prefix
                        FROM debian_repository
                        WHERE tenant_id = $1
                            AND name = $2
                        LIMIT 1
                        "#,
                        tenant_id.0,
                        repo_name,
                    )
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
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
                        VALUES (
                            $1,
                            $2,
                            $3,
                            $4,
                            $5,
                            $6,
                            $7,
                            $8,
                            $9,
                            $10,
                            $11,
                            NOW(),
                            NOW()
                        )
                        RETURNING id
                        "#,
                        repo.id,
                        req.change.distribution,
                        result.release_file.meta.description,
                        result.release_file.meta.origin,
                        result.release_file.meta.label,
                        result.release_file.meta.version,
                        result.release_file.meta.suite,
                        result.release_file.meta.codename,
                        result.release_file.contents,
                        req.clearsigned,
                        req.detachsigned,
                    )
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
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
            .fetch_optional(&mut *tx)
            .await
            .unwrap()
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
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap()
                    .id
                }
            };

            // Then, we update-or-create the Packages index of the changed package.
            // First, capture the current hash values for cleanup purposes
            let current_hashes = sqlx::query!(
                r#"
                SELECT md5sum, sha1sum, sha256sum
                FROM debian_repository_index_packages
                WHERE
                    component_id = $1
                    AND architecture = $2::debian_repository_architecture
                    AND compression IS NULL
                LIMIT 1
                "#,
                component_id,
                result.changed_packages_index.meta.architecture as _,
            )
            .fetch_optional(&mut *tx)
            .await
            .unwrap();

            match sqlx::query!(
                r#"
                SELECT id
                FROM debian_repository_index_packages
                WHERE
                    component_id = $1
                    AND architecture = $2::debian_repository_architecture
                    AND compression IS NULL
                LIMIT 1
                "#,
                component_id,
                result.changed_packages_index.meta.architecture as _,
            )
            .fetch_optional(&mut *tx)
            .await
            .unwrap()
            {
                Some(index) => {
                    // No need to check whether an update is needed - we know already
                    // that the index has changed.
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
                        result.changed_packages_index.contents.as_bytes(),
                        result.changed_packages_index.meta.size,
                        result.changed_packages_index.meta.md5sum,
                        result.changed_packages_index.meta.sha1sum,
                        result.changed_packages_index.meta.sha256sum,
                    )
                    .execute(&mut *tx)
                    .await
                    .unwrap();
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
                        result.changed_packages_index.meta.architecture as _,
                        // compression = NULL,
                        result.changed_packages_index.meta.size,
                        result.changed_packages_index.contents.as_bytes(),
                        result.changed_packages_index.meta.md5sum,
                        result.changed_packages_index.meta.sha1sum,
                        result.changed_packages_index.meta.sha256sum,
                    )
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                }
            }

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
                result.changed_package.package.sha256sum,
                component_id,
                result.changed_package.filename,
            )
            .execute(&mut *tx)
            .await
            .unwrap();

            current_hashes.map(|hashes| (hashes.md5sum, hashes.sha1sum, hashes.sha256sum))
        }
        PackageChangeAction::Remove {
            ref name,
            ref version,
            ref architecture,
        } => {
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
                name,
                version,
                architecture as _,
            )
            .fetch_optional(&mut *tx)
            .await
            .unwrap()
            .ok_or(ErrorResponse::new(
                StatusCode::NOT_FOUND,
                "COMPONENT_NOT_FOUND".to_string(),
                "component not found".to_string(),
            ))?;

            // Capture the current hash values for cleanup purposes before any changes
            let current_hashes = sqlx::query!(
                r#"
                SELECT md5sum, sha1sum, sha256sum
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
            .fetch_optional(&mut *tx)
            .await
            .unwrap();

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
            .execute(&mut *tx)
            .await
            .unwrap();

            // Update the Packages index, or delete if it's orphaned.
            if result.changed_packages_index.contents.is_empty() {
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
                .execute(&mut *tx)
                .await
                .unwrap();
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
                    "#,
                    result.changed_packages_index.contents.as_bytes(),
                    result.changed_packages_index.meta.size,
                    result.changed_packages_index.meta.md5sum,
                    result.changed_packages_index.meta.sha1sum,
                    result.changed_packages_index.meta.sha256sum,
                    component_package.component_id,
                    architecture as _,
                )
                .execute(&mut *tx)
                .await
                .unwrap();
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
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            if remaining_component_packages.count == 0 {
                sqlx::query!(
                    r#"
                    DELETE FROM debian_repository_component
                    WHERE id = $1
                "#,
                    component_package.component_id,
                )
                .execute(&mut *tx)
                .await
                .unwrap();
            }

            // We do not delete the Release, because clients may still be
            // pointing to the release file, and we don't want them to be
            // broken. The error they should get from APT is "package missing",
            // rather than "repository not found".
            current_hashes.map(|hashes| (hashes.md5sum, hashes.sha1sum, hashes.sha256sum))
        }
    };

    // Commit the transaction. At this point, the transaction may abort because
    // of a concurrent index change. This should trigger the client to retry.
    //
    // TODO(#84): Add special handling for aborts to signal the client to retry.
    tx.commit().await.unwrap();

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

    // Copy the package from its canonical storage location into the repository
    // pool.
    match req.change.action {
        PackageChangeAction::Add { .. } => {
            state
                .s3
                .copy_object()
                .bucket(&repo.s3_bucket)
                .key(format!(
                    "{}/{}",
                    repo.s3_prefix, result.changed_package.filename
                ))
                .copy_source(format!(
                    "{}/packages/{}",
                    result.changed_package.package.s3_bucket,
                    result.changed_package.package.sha256sum,
                ))
                .send()
                .await
                .unwrap();
        }
        PackageChangeAction::Remove {
            ref name,
            ref version,
            ref architecture,
        } => {
            let mut tx = state.db.begin().await.unwrap();
            sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                .execute(&mut *tx)
                .await
                .unwrap();

            // This should be safe, because even if we delete the
            // component-package, we never delete the package row, and
            // successful index generation implies the package row exists.
            let package =
                Package::query_from_meta(&mut tx, &tenant_id, name, version, architecture)
                    .await
                    .ok_or(ErrorResponse::new(
                        StatusCode::NOT_FOUND,
                        "PACKAGE_NOT_FOUND".to_string(),
                        "package not found".to_string(),
                    ))?;

            let pool_filename = package.pool_filename_in_component(&req.change.component);

            // Determine whether there exist other component-packages with the
            // same filename.
            let remaining_component_packages = sqlx::query!(
                r#"
                SELECT COUNT(*) AS "count!: i64"
                FROM
                    debian_repository_package
                    JOIN debian_repository_component_package ON debian_repository_package.id = debian_repository_component_package.package_id
                WHERE
                    debian_repository_package.tenant_id = $1
                    AND debian_repository_package.package = $2
                    AND debian_repository_package.version = $3
                    AND debian_repository_package.architecture = $4::debian_repository_architecture
                    AND debian_repository_component_package.filename = $5
                "#,
                tenant_id.0,
                name,
                version,
                architecture as _,
                pool_filename,
            )
            .fetch_one(&mut *tx)
            .await
            .unwrap();

            tx.commit().await.unwrap();

            // Delete the pool file from S3 if it's fully orphaned.
            if remaining_component_packages.count == 0 {
                state
                    .s3
                    .delete_object()
                    .bucket(&repo.s3_bucket)
                    .key(format!("{}/{}", repo.s3_prefix, pool_filename))
                    .send()
                    .await
                    .unwrap();
            }
        }
    }

    // Upload the updated package index files to standard path and all by-hash paths concurrently.
    // Index modifications are split on both sides of the release file upload:
    // - Before release files are uploaded, we upload new index contents.
    // - After release files are uploaded, we delete old index contents.
    //
    // The intention here is that the current release file _always points to valid files_.
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
            let s3 = state.s3.clone();
            let bucket = repo.s3_bucket.clone();
            let contents = result.changed_packages_index.contents.clone();
            let sha256sum = result.changed_packages_index.meta.sha256sum.clone();

            async move {
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
                            .encode(hex::decode(&sha256sum).unwrap()),
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
        state
            .s3
            .put_object()
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

    // Now we can do deletions: the release files are uploaded and are no longer pointing at the items
    // that we're about to delete.
    //
    // There are two conditions that trigger deletion:
    // - If the index is empty, we delete the Packages file and all by-hash objects, because they've been removed.
    // - Additionally, if the hashes have changed in the latest index, we delete the old by-hash objects.
    //
    // These two sets may overlap, but that's ok: we'll just dedupe them.
    let delete_via_empty_index = match result.changed_packages_index.contents.is_empty() {
        true => Vec::new(),
        false => vec![
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
        ],
    };
    let delete_via_hash_change = match old_hashes {
        None => Vec::new(),
        Some((old_md5, old_sha1, old_sha256)) => [
            (
                old_md5,
                &result.changed_packages_index.meta.md5sum,
                "MD5Sum",
            ),
            (
                old_sha1,
                &result.changed_packages_index.meta.sha1sum,
                "SHA1",
            ),
            (
                old_sha256,
                &result.changed_packages_index.meta.sha256sum,
                "SHA256",
            ),
        ]
        .into_iter()
        .filter(|(old_hash, new_hash, _)| &old_hash != new_hash)
        .map(|(old_hash, _, hash_type)| format!("{by_hash_prefix}/{hash_type}/{old_hash}"))
        .collect::<Vec<_>>(),
    };

    // Now we just dedupe the two sets and delete them in one go.
    // S3 only allows up to 1000 objects per delete request, but we're dealing with low ones of keys.
    let keys = delete_via_empty_index
        .into_iter()
        .chain(delete_via_hash_change.into_iter())
        .unique()
        .map(|key| {
            aws_sdk_s3::types::ObjectIdentifier::builder()
                .key(key)
                .build()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let delete = aws_sdk_s3::types::Delete::builder()
        .set_objects(Some(keys))
        .build()
        .unwrap();
    let deletion = state
        .s3
        .delete_objects()
        .bucket(&repo.s3_bucket)
        .delete(delete)
        .send()
        .await;
    if let Err(err) = deletion {
        tracing::error!("Failed to delete objects: {err:?}");
    }

    Ok(Json(SignIndexResponse {}))
}
