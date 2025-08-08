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
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

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
    .unwrap()
    .ok_or(ErrorResponse::not_found("repository"))?;

    // Apply the change to the database.
    let result = apply_change_to_db(&mut tx, &tenant_id, &req).await?;

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
    apply_change_to_s3(&state.s3, &repo, &req, &result).await;

    Ok(Json(SignIndexResponse {}))
}

async fn apply_change_to_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
) -> Result<PackageChangeResult, ErrorResponse> {
    // Verify the request cleartext signature.
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

    // Replay the diff onto the current state of the index. Since index
    // generation is deterministic, this should yield the same index that was
    // signed locally.
    let result =
        generate_release_file_with_change(tx, tenant_id, &req.change, req.release_ts).await?;
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
    match req.change.action {
        PackageChangeAction::Add { .. } => {
            add_package_to_db(tx, tenant_id, req, &result).await;
        }
        PackageChangeAction::Remove {
            ref name,
            ref version,
            ref architecture,
        } => {
            remove_package_from_db(
                tx,
                tenant_id,
                req,
                &result,
                name,
                version,
                architecture,
            )
            .await;
        }
    }

    Ok(result)
}

async fn add_package_to_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
    update: &PackageChangeResult,
) {
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
    .unwrap() {
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
                .unwrap();
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
    .fetch_optional(&mut **tx)
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
            .fetch_one(&mut **tx)
            .await
            .unwrap()
            .id
        }
    };

    // Then, we update-or-create the Packages index of the changed package.
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
        update.changed_packages_index.meta.architecture as _,
    )
    .fetch_optional(&mut **tx)
    .await
    .unwrap()
    {
        Some(index) => {
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
        update.changed_package.package.sha256sum,
        component_id,
        update.changed_package.filename,
    )
    .execute(&mut **tx)
    .await
    .unwrap();
}

async fn remove_package_from_db(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &TenantID,
    req: &SignIndexRequest,
    update: &PackageChangeResult,
    package: &str,
    version: &str,
    architecture: &str,
) {
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
    .execute(&mut **tx)
    .await
    .unwrap();

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
    .fetch_one(&mut **tx)
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
        .execute(&mut **tx)
        .await
        .unwrap();
    }

    // We do not delete the Release even if it's orphaned, because
    // clients may still be pointing to the release file, and we don't
    // want them to be broken. The error they should get from APT is
    // "package missing", rather than "repository not found".
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
) {
    // Copy the package from its canonical storage location into the repository
    // pool.
    match req.change.action {
        PackageChangeAction::Add { .. } => {
            s3.copy_object()
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
        PackageChangeAction::Remove { .. } => {
            // Delete the pool file from S3 if it's fully orphaned.
            if result.orphaned_pool_filename {
                s3.delete_object()
                    .bucket(&repo.s3_bucket)
                    .key(format!(
                        "{}/{}",
                        repo.s3_prefix, result.changed_package.filename
                    ))
                    .send()
                    .await
                    .unwrap();
            }
        }
    }

    // Upload the updated Packages index file.
    if result.changed_packages_index.contents.is_empty() {
        s3.delete_object()
            .bucket(&repo.s3_bucket)
            .key(format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix,
                req.change.distribution,
                result.changed_packages_index.meta.component,
                result.changed_packages_index.meta.architecture
            ))
            .send()
            .await
            .unwrap();
    } else {
        s3.put_object()
            .bucket(&repo.s3_bucket)
            .key(format!(
                "{}/dists/{}/{}/binary-{}/Packages",
                repo.s3_prefix,
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
    }
    // Upload the updated Release files. This must happen after package uploads
    // and index uploads so that all files are in place for Acquire-By-Hash.
    s3.put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/InRelease",
            repo.s3_prefix, req.change.distribution
        ))
        .content_md5(
            base64::engine::general_purpose::STANDARD
                .encode(Md5::digest(req.clearsigned.as_bytes())),
        )
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .checksum_sha256(
            base64::engine::general_purpose::STANDARD
                .encode(Sha256::digest(req.clearsigned.as_bytes())),
        )
        .body(req.clearsigned.as_bytes().to_vec().into())
        .send()
        .await
        .unwrap();
    s3.put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release",
            repo.s3_prefix, req.change.distribution
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
    s3.put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/Release.gpg",
            repo.s3_prefix, req.change.distribution
        ))
        .content_md5(
            base64::engine::general_purpose::STANDARD
                .encode(Md5::digest(req.detachsigned.as_bytes())),
        )
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .checksum_sha256(
            base64::engine::general_purpose::STANDARD
                .encode(Sha256::digest(req.detachsigned.as_bytes())),
        )
        .body(req.detachsigned.as_bytes().to_vec().into())
        .send()
        .await
        .unwrap();
}
