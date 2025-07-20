use aws_sdk_s3::config::BehaviorVersion;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    handler::Handler,
    routing::{delete, get, post},
};
use sha2::{Digest as _, Sha256};
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::{info, trace, warn};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

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

    // Initialize database.
    let db_url = std::env::var("ATTUNE_DATABASE_URL").expect("ATTUNE_DATABASE_URL not set");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");

    // Initialize special single-tenant user.
    sqlx::query!(
        r#"
        INSERT INTO attune_tenant (id, display_name, subdomain, created_at, updated_at)
        VALUES (1, 'LOCAL_TENANT', '', NOW(), NOW())
        ON CONFLICT DO NOTHING;
        "#
    )
    .execute(&db)
    .await
    .unwrap();
    // If $ATTUNE_API_TOKEN is set, initialize the special single-tenant API
    // token.
    match std::env::var("ATTUNE_API_TOKEN") {
        Ok(api_token) => {
            let mut tx = db.begin().await.unwrap();
            sqlx::query!("DELETE FROM attune_tenant_api_token WHERE tenant_id = 1;")
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query!(
                r#"
                INSERT INTO attune_tenant_api_token (tenant_id, name, token, created_at, updated_at)
                VALUES (1, 'LOCAL_TENANT_API_TOKEN', $1, NOW(), NOW())
                ON CONFLICT DO NOTHING;
                "#,
                Sha256::digest(api_token).as_slice().to_vec(),
            )
            .execute(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        Err(e) => match e {
            std::env::VarError::NotPresent => {
                warn!(
                    "$ATTUNE_API_TOKEN is not set, skipping single-tenant API token initialization"
                )
            }
            std::env::VarError::NotUnicode(os_string) => {
                panic!("$ATTUNE_API_TOKEN is not valid Unicode: {os_string:?}")
            }
        },
    }

    // Initialize AWS S3 client.
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let config = aws_sdk_s3::config::Builder::from(&config).build();
    trace!(?config, "inferred AWS S3 configuration from environment");
    let s3 = aws_sdk_s3::Client::from_conf(config);
    let s3_bucket_name =
        std::env::var("ATTUNE_S3_BUCKET_NAME").unwrap_or("attune-dev-0".to_string());

    // Configure routes.
    let api = Router::new()
        .route(
            "/repositories",
            get(attune::api::repo::list).post(attune::api::repo::create),
        )
        .route(
            "/repositories/{repository_id}",
            get(attune::api::repo::status),
        )
        .route(
            "/repositories/{repository_id}/indexes",
            get(attune::api::sign::generate_indexes),
        )
        .route(
            "/repositories/{repository_id}/sync",
            post(attune::api::sign::sync_repository),
        )
        .route(
            "/repositories/{repository_id}/packages",
            get(attune::api::pkg::list)
                .post(attune::api::pkg::add.layer(DefaultBodyLimit::disable())),
        )
        .route(
            "/repositories/{repository_id}/packages/{package_id}",
            delete(attune::api::pkg::remove),
        );
    let app = Router::new()
        .nest("/api/v0", api)
        .layer(TraceLayer::new_for_http())
        .with_state(attune::api::ServerState {
            db,
            s3,
            s3_bucket_name,
        });

    // Start server.
    info!(address = "0.0.0.0:3000", "starting server");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .unwrap();
}

async fn shutdown() {
    signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("could not install SIGTERM handler")
        .recv()
        .await;
}
