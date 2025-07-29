use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
};
use bytes::Bytes;
use debian_packaging::{
    binary_package_control::BinaryPackageControlFile,
    deb::reader::{BinaryPackageEntry, BinaryPackageReader, ControlTarFile},
};
use digest::Digest as _;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::Sha256;
use sqlx::{Executor, Postgres, types::JsonValue};
use tracing::{debug_span, instrument};

use crate::{api::ErrorResponse, auth::TenantID, server::ServerState};

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageUploadResponse {
    pub sha256sum: String,
}

#[axum::debug_handler]
#[instrument(skip(state, multipart))]
pub async fn handler(
    State(state): State<ServerState>,
    tenant_id: TenantID,
    mut multipart: Multipart,
) -> Result<Json<PackageUploadResponse>, ErrorResponse> {
    // TODO: We currently hold the entire package in memory. This works for now,
    // but we could theoretically rebuild this handler to be fully streaming
    // (from the request into S3 object storage, while parsing needed values
    // along the way).

    // Parse the uploaded package.
    let field = multipart
        .next_field()
        .await
        .unwrap()
        .expect("expected a file");
    let name = field.name().unwrap().to_string();
    if name != "file" {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "COULD_NOT_PARSE_UPLOAD".to_string(),
            format!("expected field named \"file\", got {:?}", name),
        ));
    }

    // Parse Debian package for control fields.
    let value = field.bytes().await.unwrap();
    let control_file = parse_debian_package(&value).await;
    let hashes = calculate_hashes(&value).await;
    let size = value.len() as i64;

    // Check that there are no more fields.
    let None = multipart.next_field().await.unwrap() else {
        return Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "COULD_NOT_PARSE_UPLOAD".to_string(),
            format!("unexpected unknown field {:?}", name),
        ));
    };

    // Begin database transaction.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Insert the package row into the database. At this point, integrity checks
    // may cause the upload to fail.
    insert_package(
        &mut *tx,
        tenant_id,
        &state.s3_bucket_name,
        control_file,
        &hashes,
        size,
    )
    .await
    .unwrap();

    // Upload the package to S3.
    state
        .s3
        .put_object()
        .bucket(&state.s3_bucket_name)
        .key(format!("packages/{}", hashes.sha256sum))
        .body(value.into())
        .send()
        .await
        .unwrap();

    // Commit the transaction. This must occur after the package is uploaded to
    // S3 so that a handler crash does not leave us in a state where the row
    // exists but the file is missing.
    tx.commit().await.unwrap();

    Ok(Json(PackageUploadResponse {
        sha256sum: hashes.sha256sum,
    }))
}

#[instrument(skip(value))]
async fn parse_debian_package(value: &Bytes) -> BinaryPackageControlFile<'static> {
    let mut reader = BinaryPackageReader::new(value.as_ref()).unwrap();
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
}

#[derive(Debug)]
struct Hashes {
    sha256sum: String,
    sha1sum: String,
    md5sum: String,
}

#[instrument(skip(value))]
async fn calculate_hashes(value: &Bytes) -> Hashes {
    // TODO: Can we make this faster? Parallelism? Streaming? Asynchrony?
    let sha256sum = debug_span!("sha256sum").in_scope(|| hex::encode(Sha256::digest(&value)));
    let sha1sum = debug_span!("sha1sum").in_scope(|| hex::encode(Sha1::digest(&value)));
    let md5sum = debug_span!("md5sum").in_scope(|| hex::encode(Md5::digest(&value)));
    Hashes {
        sha256sum,
        sha1sum,
        md5sum,
    }
}

#[instrument(skip(executor, control_file))]
async fn insert_package<'c, E>(
    executor: E,
    tenant_id: TenantID,
    s3_bucket_name: &str,
    control_file: BinaryPackageControlFile<'static>,
    hashes: &Hashes,
    size: i64,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    // Compute fields.
    let package_name = control_file.package().unwrap();
    let version = control_file.version().unwrap().to_string();
    let architecture = control_file.architecture().unwrap();
    let md5sum = &hashes.md5sum;
    let sha1sum = &hashes.sha1sum;
    let sha256sum = &hashes.sha256sum;
    let paragraph = JsonValue::Object(
        control_file
            .as_str_hash_map()
            .into_iter()
            .map(|(k, v)| (k.to_string(), JsonValue::String(v.to_string())))
            .collect(),
    );

    // Run insertion.
    let inserted = sqlx::query!(
        r#"
        INSERT INTO debian_repository_package (
            tenant_id,
            s3_bucket,

            package,
            version,
            architecture,

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

            size,
            md5sum,
            sha1sum,
            sha256sum,

            created_at,
            updated_at
        )
        VALUES (
            $1,
            $2,

            $3,
            $4,
            $5::debian_repository_architecture,

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

            NOW(),
            NOW()
        )
        RETURNING id
        "#,
        tenant_id.0,
        s3_bucket_name,
        package_name,
        &version,
        architecture as _,
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
        size,
        md5sum,
        sha1sum,
        sha256sum,
    )
    .fetch_one(executor)
    .await?;
    Ok(inserted.id)
}
