use aws_sdk_s3::types::ChecksumAlgorithm;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use base64::Engine as _;
use md5::{Digest as _, Md5};
use pgp::composed::{
    CleartextSignedMessage, Deserializable as _, SignedPublicKey, StandaloneSignature,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use time::OffsetDateTime;
use tracing::instrument;

use crate::{
    api::ErrorResponse,
    auth::TenantID,
    server::{
        ServerState,
        repo::{
            decode_repo_name,
            index::{PackageChange, generate_release_file_with_change},
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
#[serde(tag = "action", rename_all = "lowercase")]
pub enum IndexChange {
    Add,
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
    // The repository name in the path is percent-encoded.
    let repo_name = decode_repo_name(&repo_name)?;
    if repo_name != req.change.repository {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "REPOSITORY_MISMATCH".to_string(),
            "repository name in path does not match repository name in request".to_string(),
        ));
    }

    // Verify the index signatures.
    let (public_key, _headers) = SignedPublicKey::from_string(&req.public_key_cert)
        .expect("could not parse public key certificate");
    if let Err(e) = public_key.verify() {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "PUBLIC_KEY_VERIFICATION_FAILED".to_string(),
            format!("could not verify public key: {}", e),
        ));
    }
    let (clearsigned, _headers) = CleartextSignedMessage::from_string(&req.clearsigned)
        .expect("could not parse clearsigned index");
    if let Err(e) = clearsigned.verify(&public_key) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "CLEARSIGN_VERIFICATION_FAILED".to_string(),
            format!("could not verify clearsigned index: {}", e),
        ));
    }
    let contents = clearsigned.text();
    let (detachsigned, _headers) = StandaloneSignature::from_string(&req.detachsigned)
        .expect("could not parse detached signature");
    if let Err(e) = detachsigned.verify(&public_key, contents.as_bytes()) {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "DETACHED_SIGNATURE_VERIFICATION_FAILED".to_string(),
            format!("could not verify detached signature: {}", e),
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

    // Compare the replayed index with the signed index. Accept the signature if
    // the index contents match. Otherwise, return an error.
    if result.release_file.contents != contents {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "INDEX_CONTENTS_MISMATCH".to_string(),
            "index contents do not match".to_string(),
        ));
    }

    // Save the new state to the database.
    //
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
            if release.description != result.release_file.release.description ||
                release.origin != result.release_file.release.origin ||
                release.label != result.release_file.release.label ||
                release.version != result.release_file.release.version ||
                release.suite != result.release_file.release.suite ||
                release.codename != result.release_file.release.codename ||
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
                    result.release_file.release.description,
                    result.release_file.release.origin,
                    result.release_file.release.label,
                    result.release_file.release.version,
                    result.release_file.release.suite,
                    result.release_file.release.codename,
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
                result.release_file.release.description,
                result.release_file.release.origin,
                result.release_file.release.label,
                result.release_file.release.version,
                result.release_file.release.suite,
                result.release_file.release.codename,
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
        result.changed_packages_index.architecture as _,
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
                result.changed_packages_index_contents.as_bytes(),
                result.changed_packages_index.size,
                result.changed_packages_index.md5sum,
                result.changed_packages_index.sha1sum,
                result.changed_packages_index.sha256sum,
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
                result.changed_packages_index.architecture as _,
                // compression = NULL,
                result.changed_packages_index.size,
                result.changed_packages_index_contents.as_bytes(),
                result.changed_packages_index.md5sum,
                result.changed_packages_index.sha1sum,
                result.changed_packages_index.sha256sum,
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

    // Commit the transaction. At this point, the transaction may abort because
    // of a concurrent index change. This should trigger the client to retry.
    //
    // TODO: Add special handling for aborts to signal the client to retry.
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
            result.changed_package.package.s3_bucket, result.changed_package.package.sha256sum,
        ))
        .send()
        .await
        .unwrap();

    // Upload the updated Packages index file.
    state
        .s3
        .put_object()
        .bucket(&repo.s3_bucket)
        .key(format!(
            "{}/dists/{}/{}/binary-{}/Packages",
            repo.s3_prefix,
            req.change.distribution,
            result.changed_packages_index.component,
            result.changed_packages_index.architecture
        ))
        .content_md5(
            base64::engine::general_purpose::STANDARD.encode(Md5::digest(
                result.changed_packages_index_contents.as_bytes(),
            )),
        )
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .checksum_sha256(
            base64::engine::general_purpose::STANDARD
                .encode(hex::decode(&result.changed_packages_index.sha256sum).unwrap()),
        )
        .body(
            result
                .changed_packages_index_contents
                .as_bytes()
                .to_vec()
                .into(),
        )
        .send()
        .await
        .unwrap();

    // Upload the updated Release files. This must happen last to take advantage
    // of Acquire-By-Hash.
    state
        .s3
        .put_object()
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
    state
        .s3
        .put_object()
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
    state
        .s3
        .put_object()
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

    Ok(Json(SignIndexResponse {}))
}
