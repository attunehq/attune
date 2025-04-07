use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Query, State},
    handler::Handler,
    routing::{get, post},
};
use debian_packaging::deb::reader::{BinaryPackageEntry, BinaryPackageReader};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

#[tokio::main]
async fn main() {
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

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL not set");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");

    let api = Router::new()
        .route(
            "/repositories",
            get(list_repositories).post(create_repository),
        )
        .route("/releases", get(list_releases).post(create_release))
        .route(
            "/packages",
            post(add_package.layer(DefaultBodyLimit::disable())),
        );
    let app = Router::new()
        .nest("/api/v0", api)
        .layer(TraceLayer::new_for_http())
        .with_state(pool);

    println!("Listening on http://0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[derive(Debug, Deserialize)]
struct CreateReleaseRequest {
    repository_id: i64,
    #[serde(flatten)]
    definition: CreateReleaseDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CreateReleaseDefinition {
    FromExisting {
        from: i64,
        origin: Option<String>,
        label: Option<String>,
        suite: Option<String>,
        codename: Option<String>,
        description: Option<String>,
    },
    Empty {
        origin: String,
        label: String,
        suite: String,
        codename: String,
        description: String,
    },
}

#[derive(Deserialize, Serialize)]
struct ReleaseSummary {
    id: i64,
    origin: String,
    label: String,
    suite: String,
    codename: String,
    date: String,
    description: String,
    active: bool,
    signed: bool,
    stale: bool,
}

// #[axum::debug_handler]
async fn create_release(
    State(pool): State<sqlx::PgPool>,
    Json(request): Json<CreateReleaseRequest>,
) -> Json<ReleaseSummary> {
    Json(match request.definition {
        CreateReleaseDefinition::FromExisting {
            from,
            origin,
            label,
            suite,
            codename,
            description,
        } => {
            todo!()
        }
        CreateReleaseDefinition::Empty {
            origin,
            label,
            suite,
            codename,
            description,
        } => {
            let release = sqlx::query!(
                r#"
                INSERT INTO debian_repository_release (repository_id, origin, label, suite, codename, description, stale)
                VALUES ($1, $2, $3, $4, $5, $6, false)
                RETURNING *"#,
                request.repository_id,
                origin,
                label,
                suite,
                codename,
                description
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            ReleaseSummary {
                id: release.id,
                origin: release.origin,
                label: release.label,
                suite: release.suite,
                codename: release.codename,
                date: release.updated_at.to_string(),
                description: release.description,
                active: false,
                signed: false,
                stale: false,
            }
        }
    })
}

async fn list_releases(
    State(pool): State<sqlx::PgPool>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Vec<ReleaseSummary>> {
    let repository_id = params.get("repository_id").unwrap().parse::<i64>().unwrap();

    let releases = sqlx::query!(
        r#"
        SELECT
            debian_repository_release.id,
            debian_repository_release.origin,
            debian_repository_release.label,
            debian_repository_release.suite,
            debian_repository_release.codename,
            debian_repository_release.updated_at,
            debian_repository_release.description,
            (debian_repository_release.id = debian_repository.active_release_id) AS active,
            debian_repository_release.pgp_signature IS NOT NULL AS signed,
            debian_repository_release.stale
        FROM debian_repository_release
        JOIN debian_repository ON debian_repository_release.repository_id = debian_repository.id
        WHERE debian_repository.id = $1"#,
        repository_id,
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    Json(
        releases
            .into_iter()
            .map(|r| ReleaseSummary {
                id: r.id,
                origin: r.origin,
                label: r.label,
                suite: r.suite,
                codename: r.codename,
                date: r.updated_at.to_string(),
                description: r.description,
                active: r.active.unwrap_or(false),
                signed: r.signed.unwrap_or(false),
                stale: r.stale,
            })
            .collect(),
    )
}

async fn create_repository() -> Json<Repository> {
    todo!()
}

#[derive(Serialize)]
struct Repository {
    id: i64,
    uri: String,
    distribution: String,
    active_release_id: Option<i64>,
}

async fn list_repositories(State(pool): State<sqlx::PgPool>) -> Json<Vec<Repository>> {
    let repositories =
        sqlx::query!("SELECT id, uri, distribution, active_release_id FROM debian_repository",)
            .fetch_all(&pool)
            .await
            .unwrap();
    Json(
        repositories
            .into_iter()
            .map(|r| Repository {
                id: r.id,
                uri: r.uri,
                distribution: r.distribution,
                active_release_id: r.active_release_id,
            })
            .collect(),
    )
}

#[derive(Deserialize, Serialize)]
struct AddPackageResponse {}

#[axum::debug_handler]
async fn add_package(
    State(pool): State<sqlx::PgPool>,
    Query(params): Query<HashMap<String, String>>,
    mut multipart: Multipart,
) -> Json<AddPackageResponse> {
    // Parse request.
    let release_id = params.get("release_id").unwrap().parse::<i64>().unwrap();
    let component = params.get("component").unwrap().to_string();

    // Parse uploaded body file.
    let Some(field) = multipart.next_field().await.unwrap() else {
        panic!("expected a file");
    };
    let name = field.name().unwrap().to_string();
    if name != "file" {
        panic!("unexpected field name: {}", name);
    }

    // Parse Debian package for control fields.
    let value = field.bytes().await.unwrap();
    let mut reader = BinaryPackageReader::new(value.as_ref()).unwrap();
    {
        let header_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::DebianBinary { .. } = header_entry else {
            panic!("expected a Debian binary package")
        };
        let control_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Control { .. } = control_entry else {
            panic!("expected a control file")
        };
        let data_entry = reader.next_entry().unwrap().unwrap();
        let BinaryPackageEntry::Data { .. } = data_entry else {
            panic!("expected a data file")
        };
    }

    // Now that we're sure it's a valid Debian package, upload it to R2 into the
    // pool of the release under its SHA256 content address.

    // Once the upload is complete, add a record to the database.

    println!("{}: {}", name, value.len());

    let None = multipart.next_field().await.unwrap() else {
        panic!("expected no more fields");
    };

    Json(AddPackageResponse {})
}
