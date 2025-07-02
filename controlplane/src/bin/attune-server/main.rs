use aws_sdk_s3::config::BehaviorVersion;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    handler::Handler,
    routing::{get, post},
};
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::debug;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

mod api;

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

    // Initialize service dependencies.
    let db_url = std::env::var("ATTUNE_DATABASE_URL").expect("ATTUNE_DATABASE_URL not set");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let config = aws_sdk_s3::config::Builder::from(&config).build();
    debug!(?config, "inferred AWS S3 configuration from environment");
    let s3 = aws_sdk_s3::Client::from_conf(config);
    let s3_bucket_name =
        std::env::var("ATTUNE_S3_BUCKET_NAME").unwrap_or("attune-dev-0".to_string());

    // Configure routes.
    let api = Router::new()
        .route(
            "/repositories",
            get(api::list_repositories).post(api::create_repository),
        )
        .route("/repositories/{repository_id}", get(api::repository_status))
        .route(
            "/repositories/{repository_id}/indexes",
            get(api::generate_indexes),
        )
        .route(
            "/repositories/{repository_id}/sync",
            post(api::sync_repository),
        )
        .route(
            "/repositories/{repository_id}/packages",
            get(api::list_packages)
                .delete(api::remove_package)
                .post(api::add_package.layer(DefaultBodyLimit::disable())),
        );
    let app = Router::new()
        .nest("/api/v0", api)
        .layer(TraceLayer::new_for_http())
        .with_state(api::ServerState {
            db,
            s3,
            s3_bucket_name,
        });

    // Start server.
    println!("Listening on http://0.0.0.0:3000");
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
