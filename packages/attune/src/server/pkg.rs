use std::collections::HashMap;

use crate::auth::TenantID;
use crate::{server::ServerState, auth};
use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
};
use debian_packaging::deb::reader::{BinaryPackageEntry, BinaryPackageReader, ControlTarFile};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::types::JsonValue;
use tracing::{Instrument, debug_span, instrument};

#[derive(Deserialize, Serialize)]
pub struct Package {
    id: i64,
    package: String,
    version: String,
    architecture: String,
    component: String,
}

#[axum::debug_handler]
#[instrument(skip(state, multipart))]
pub async fn add(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(release_id): Path<u64>,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Result<Json<Package>, (axum::http::StatusCode, &'static str)> {
    // FIXME: Should this whole handler be in a transaction? Does it matter?
    // Will holding the transaction open for the duration of the upload cause
    // problems?

    // Check that the release exists and the user has permission to modify it.
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    // Parse request.
    let component = params.get("component").unwrap().to_string();

    // Parse uploaded body file.
    let field = multipart
        .next_field()
        .await
        .unwrap()
        .expect("expected a file");
    let name = field.name().unwrap().to_string();
    if name != "file" {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "unexpected field named \"file\"\n",
        ));
    }

    // TODO: Is there a way to implement this function body with streaming
    // instead of holding the whole file in memory?

    // Parse Debian package for control fields.
    let value = field.bytes().await.unwrap();
    let mut reader = BinaryPackageReader::new(value.as_ref()).unwrap();
    let control_file = debug_span!("parse_debian_package").in_scope(|| {
        let header_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::DebianBinary(_) = header_entry else {
            panic!("expected a Debian binary package")
        };
        let control_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Control(mut control_reader) = control_entry else {
            panic!("expected a control file")
        };
        let mut control_entries = control_reader.entries().unwrap();
        let control_file = loop {
            let (_, control_tar_file) = control_entries
                .next()
                .unwrap()
                .unwrap()
                .to_control_file()
                .unwrap();
            if let ControlTarFile::Control(control_file) = control_tar_file {
                break control_file;
            }
        };
        // TODO: Parse file paths for building Contents index.
        let data_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Data(_) = data_entry else {
            panic!("expected a data file")
        };
        control_file
    });
    let architecture = control_file.architecture().unwrap();

    // TODO: Check if the package already exists before we spend time computing
    // hashes. If so, provide a nice error message.

    // Compute hashes.
    //
    // TODO: Can we make this faster?
    let (sha256sum, sha1sum, md5sum) = debug_span!("compute_hashes").in_scope(|| {
        let sha256sum = debug_span!("sha256sum").in_scope(|| hex::encode(Sha256::digest(&value)));
        let sha1sum = debug_span!("sha1sum").in_scope(|| hex::encode(Sha1::digest(&value)));
        let md5sum = debug_span!("md5sum").in_scope(|| hex::encode(Md5::digest(&value)));
        (sha256sum, sha1sum, md5sum)
    });
    let size = value.len() as i64;

    // Now that we're sure it's a valid Debian package, upload it into the pool.
    let package_name = control_file.package().unwrap();
    let package_name_start = package_name.chars().next().unwrap();
    let version = control_file.version().unwrap().to_string();
    let pool_filename = format!(
        "pool/{component}/{package_name_start}/{package_name}/{package_name}_{version}_{architecture}.deb"
    );

    let span = debug_span!("upload_to_pool");
    async {
        let repo = sqlx::query!(
            r#"
                SELECT
                    debian_repository.s3_bucket,
                    debian_repository.s3_prefix
                FROM debian_repository
                    JOIN debian_repository_release ON debian_repository_release.repository_id = debian_repository.id
                WHERE debian_repository_release.id = $1
            "#,
            release_id as i64
        )
        .fetch_one(&state.db)
        .await
        .unwrap();

        state
            .s3
            .put_object()
            .bucket(&repo.s3_bucket)
            .key(format!("{}/staging/{pool_filename}", repo.s3_prefix))
            .body(value.into())
            .send()
            .await
            .unwrap();
    }
    .instrument(span)
    .await;

    // Once the upload is complete, add a record to the database.
    let span = debug_span!("add_to_database");
    let package_row = async {
        let mut tx = state.db.begin().await.unwrap();
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();

        // Find or create the release component.
        let component_row = sqlx::query!(
            "SELECT id FROM debian_repository_component WHERE name = $1 AND release_id = $2",
            component,
            release_id as i64
        )
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        let component_id = match component_row {
            Some(c) => c.id,
            None => {
                sqlx::query!(
                    r#"
                        INSERT INTO debian_repository_component (
                            name,
                            release_id,
                            updated_at
                        ) VALUES ($1, $2, NOW()) RETURNING id
                    "#,
                    component,
                    release_id as i64,
                )
                .fetch_one(&mut *tx)
                .await
                .unwrap()
                .id
            }
        };

        // Insert the package row.
        let paragraph = JsonValue::Object(
            control_file
                .as_str_hash_map()
                .into_iter()
                .map(|(k, v)| (k.to_string(), JsonValue::String(v.to_string())))
                .collect(),
        );
        let package_row = sqlx::query!(
            r#"
            INSERT INTO debian_repository_package (
                component_id,
                architecture,

                staging_status,

                package,
                version,
                priority,
                section,
                installed_size,
                maintainer,
                description,
                homepage,

                paragraph,

                depends,
                recommends,
                conflicts,
                provides,
                replaces,

                filename,
                size,
                md5sum,
                sha1sum,
                sha256sum,

                created_at,
                updated_at
            )
            VALUES (
                $1,
                $2::debian_repository_architecture,
                $3::debian_repository_package_staging_status,
                $4,
                $5,
                $6,
                $7,
                $8,
                $9,
                $10,
                $11,
                $12,
                $13,
                $14,
                $15,
                $16,
                $17,
                $18,
                $19,
                $20,
                $21,
                $22,
                NOW(),
                NOW()
            )
            RETURNING id
            "#,
            component_id,
            architecture as _,
            "add" as _,
            package_name,
            &version,
            control_file.priority(),
            control_file.section(),
            control_file.installed_size().map(|s| s.unwrap() as i64),
            control_file.maintainer().unwrap(),
            control_file.description().unwrap(),
            control_file.homepage(),
            paragraph,
            control_file.depends().map(|d| d.unwrap().to_string()),
            control_file.recommends().map(|d| d.unwrap().to_string()),
            control_file
                .field_dependency_list("conflicts")
                .map(|d| d.unwrap().to_string()),
            control_file
                .field_dependency_list("provides")
                .map(|d| d.unwrap().to_string()),
            control_file
                .field_dependency_list("replaces")
                .map(|d| d.unwrap().to_string()),
            &pool_filename,
            size,
            md5sum,
            sha1sum,
            sha256sum,
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        package_row
    }
    .instrument(span)
    .await;

    let None = multipart.next_field().await.unwrap() else {
        panic!("expected no more fields");
    };

    Ok(Json(Package {
        id: package_row.id,
        package: package_name.to_string(),
        version,
        architecture: architecture.to_string(),
        component,
    }))
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn list(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path(release_id): Path<u64>,
) -> Result<Json<Vec<Package>>, (axum::http::StatusCode, &'static str)> {
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    let packages = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.id,
            debian_repository_package.package,
            debian_repository_package.version,
            debian_repository_package.architecture::TEXT AS "architecture!: String",
            debian_repository_component.name AS component
        FROM
            debian_repository_release
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE debian_repository_release.id = $1
            AND debian_repository_package.staging_status IS NULL
        ORDER BY debian_repository_package.id ASC
        "#,
        release_id as i64,
    )
    .map(|row| Package {
        id: row.id,
        package: row.package,
        version: row.version,
        architecture: row.architecture,
        component: row.component,
    })
    .fetch_all(&state.db)
    .await
    .unwrap();

    Ok(Json(packages))
}

#[axum::debug_handler]
#[instrument(skip(state))]
pub async fn remove(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    Path((release_id, package_id)): Path<(u64, i64)>,
) -> Result<Json<Package>, (axum::http::StatusCode, &'static str)> {
    // Check that the release exists and the user has permission to modify it.
    auth::tenant_owns_release(&state.db, tenant_id, release_id).await?;

    // Get the package information before marking it for removal.
    let package = sqlx::query!(
        r#"
        SELECT
            debian_repository_package.id,
            debian_repository_package.package,
            debian_repository_package.version,
            debian_repository_package.architecture::TEXT AS "architecture!: String",
            debian_repository_component.name AS component
        FROM
            debian_repository_release
            JOIN debian_repository_component ON debian_repository_component.release_id = debian_repository_release.id
            JOIN debian_repository_package ON debian_repository_package.component_id = debian_repository_component.id
        WHERE
            debian_repository_release.id = $1 AND
            debian_repository_package.id = $2
        "#,
        release_id as i64,
        package_id
    )
    .fetch_optional(&state.db)
    .await
    .unwrap();

    let Some(package) = package else {
        return Err((axum::http::StatusCode::NOT_FOUND, "Package not found\n"));
    };

    // Mark the package for removal by setting its staging_status to "remove".
    sqlx::query!(
        r#"
        UPDATE debian_repository_package
        SET
            staging_status = 'remove',
            updated_at = NOW()
        WHERE id = $1
        "#,
        package_id
    )
    .execute(&state.db)
    .await
    .unwrap();

    Ok(Json(Package {
        id: package.id,
        package: package.package,
        version: package.version,
        architecture: package.architecture,
        component: package.component,
    }))
}
