use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing::debug;
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
        .route("/releases", post(create_release))
        .route(
            "/repositories",
            get(list_repositories).post(create_repository),
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
            let mut tx = pool.begin().await.unwrap();
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
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
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

async fn list_releases() -> Json<Vec<ReleaseSummary>> {
    todo!()
}

#[derive(Deserialize, Serialize)]
struct Release {
    // Include the summary, but also include details like components and packages

    // id: i64,
    // origin: String,
    // label: String,
    // suite: String,
    // codename: String,
    // date: String,
    // description: String,
    // active: bool,
    // signed: bool,
    // stale: bool,
}

async fn get_release() -> Json<Release> {
    todo!()
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
