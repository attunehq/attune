use std::collections::HashMap;

use aws_sdk_s3::config::BehaviorVersion;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    handler::Handler,
    routing::{get, post},
};
use debian_packaging::deb::reader::{BinaryPackageEntry, BinaryPackageReader, ControlTarFile};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::types::JsonValue;
use tower_http::trace::TraceLayer;
use tracing::{Instrument, debug_span, instrument};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

#[derive(Clone, Debug)]
struct ServerState {
    db: sqlx::PgPool,
    s3: aws_sdk_s3::Client,
}

#[tokio::main]
async fn main() {
    // Initialize tracing.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Initialize upstream dependencies.
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL not set");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let s3 = aws_sdk_s3::Client::new(&config);

    // Configure routes.
    let api = Router::new()
        .route(
            "/repositories",
            get(list_repositories).post(create_repository),
        )
        .route("/repositories/{repository_id}", get(repository_status))
        .route("/repositories/{repository_id}/sync", post(sync_repository))
        .route(
            "/repositories/{repository_id}/packages",
            get(list_packages)
                .delete(remove_package)
                .post(add_package.layer(DefaultBodyLimit::disable())),
        );
    let app = Router::new()
        .nest("/api/v0", api)
        .layer(TraceLayer::new_for_http())
        .with_state(ServerState { db, s3 });

    // Start server.
    println!("Listening on http://0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[derive(Serialize)]
struct RepositoryResponse {
    id: i64,
    uri: String,
    distribution: String,
}

#[derive(Deserialize, Debug)]
struct CreateRepositoryRequest {
    uri: String,
    distribution: String,
    origin: String,
    label: String,
    suite: String,
    codename: String,
    description: String,
}

async fn create_repository(
    State(state): State<ServerState>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Json<RepositoryResponse> {
    let created = sqlx::query!(
        r#"
        INSERT INTO debian_repository (
            uri,
            distribution,
            origin,
            label,
            suite,
            codename,
            description
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
        payload.uri,
        payload.distribution,
        payload.origin,
        payload.label,
        payload.suite,
        payload.codename,
        payload.description,
    )
    .fetch_one(&state.db)
    .await
    .unwrap();
    Json(RepositoryResponse {
        id: created.id,
        uri: payload.uri,
        distribution: payload.distribution,
    })
}

async fn list_repositories(State(state): State<ServerState>) -> Json<Vec<RepositoryResponse>> {
    let repositories = sqlx::query!("SELECT id, uri, distribution FROM debian_repository")
        .fetch_all(&state.db)
        .await
        .unwrap();
    Json(
        repositories
            .into_iter()
            .map(|r| RepositoryResponse {
                id: r.id,
                uri: r.uri,
                distribution: r.distribution,
            })
            .collect(),
    )
}

async fn repository_status() -> Json<RepositoryResponse> {
    todo!()
}

async fn sync_repository() -> Json<RepositoryResponse> {
    todo!()
}

#[derive(Deserialize, Serialize)]
struct PackageResponse {
    id: i64,
    package: String,
    version: String,
    architecture: String,
}

#[derive(sqlx::Type)]
#[sqlx(type_name = "debian_repository_package_staging_status")]
#[sqlx(rename_all = "lowercase")]
enum DebianRepositoryPackageStagingStatus {
    Add,
    Remove,
}

#[axum::debug_handler]
#[instrument(skip(state, multipart))]
async fn add_package(
    State(state): State<ServerState>,
    Path(repository_id): Path<u64>,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Json<PackageResponse> {
    // Parse request.
    let component = params.get("component").unwrap().to_string();

    // Parse uploaded body file.
    let Some(field) = multipart.next_field().await.unwrap() else {
        panic!("expected a file");
    };
    let name = field.name().unwrap().to_string();
    if name != "file" {
        panic!("unexpected field name: {}", name);
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
            match control_tar_file {
                ControlTarFile::Control(control_file) => {
                    break control_file;
                }
                _ => {}
            }
        };
        let data_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Data(_) = data_entry else {
            panic!("expected a data file")
        };
        control_file
    });
    let architecture = control_file.architecture().unwrap();

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
    let key = format!("armor-dev-1/{pool_filename}");
    async {
        state
            .s3
            .put_object()
            .bucket("armor-dev-1")
            .key(key)
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
        let arch_row = sqlx::query!(
            "SELECT id FROM debian_repository_architecture WHERE name = $1",
            architecture
        )
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        let arch_id = match arch_row {
            Some(arch) => arch.id,
            None => {
                sqlx::query!(
                    "INSERT INTO debian_repository_architecture (name, repository_id) VALUES ($1, $2) RETURNING id",
                    architecture,
                    repository_id as i64
                )
                .fetch_one(&mut *tx)
                .await
                .unwrap()
                .id
            }
        };
        let component_row = sqlx::query!(
            "SELECT id FROM debian_repository_component WHERE name = $1",
            component
        )
        .fetch_optional(&mut *tx)
        .await
        .unwrap();
        let component_id = match component_row {
            Some(c) => c.id,
            None => {
                sqlx::query!(
                    "INSERT INTO debian_repository_component (name, repository_id) VALUES ($1, $2) RETURNING id",
                    component,
                    repository_id as i64
                )
                .fetch_one(&mut *tx)
                .await
                .unwrap()
                .id
            }
        };
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
                repository_id,
                architecture_id,
                component_id,

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
                sha256sum
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
            RETURNING id
            "#,
            repository_id as i64,
            arch_id,
            component_id,
            DebianRepositoryPackageStagingStatus::Add as DebianRepositoryPackageStagingStatus,
            // DebianRepositoryPackageStagingStatus::Add as _,
            // "add" as _,
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
            control_file.field_dependency_list("conflicts").map(|d| d.unwrap().to_string()),
            control_file.field_dependency_list("provides").map(|d| d.unwrap().to_string()),
            control_file.field_dependency_list("replaces").map(|d| d.unwrap().to_string()),
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

    Json(PackageResponse {
        id: package_row.id,
        package: package_name.to_string(),
        version,
        architecture: architecture.to_string(),
    })
}

async fn list_packages() -> Json<Vec<PackageResponse>> {
    todo!()
}

async fn remove_package() -> Json<PackageResponse> {
    todo!()
}
