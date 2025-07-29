use aws_sdk_s3::config::BehaviorVersion;
use clap::Parser;
use tokio::signal;
use tracing::{info, trace};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

/// Attune control plane server, community edition
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser)]
#[command(name = "attune-server", max_term_width = 80)]
struct Args {
    /// Postgres database URL for Attune control plane.
    #[arg(long, env = "ATTUNE_DATABASE_URL")]
    db_url: String,
    /// Name of S3 bucket for newly created repositories.
    ///
    /// Note that repositories are associated with an S3 bucket on creation, so
    /// previously created repositories will continue to be associated with
    /// their original S3 buckets.
    #[arg(long, env = "ATTUNE_S3_BUCKET_NAME", default_value = "attune-dev-0")]
    s3_bucket_name: String,
    /// API token for the default user.
    ///
    /// If set, will be used for the default user's API token. If not set,
    /// the default user will not have an API token configured.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    default_api_token: Option<String>,
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

    tracing::info!("minor change to trigger a ci rebuild");

    // Parse CLI arguments.
    let args = Args::parse();

    // Initialize database.
    let db_url = args.db_url;
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");

    // Initialize AWS S3 client.
    let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let config = aws_sdk_s3::config::Builder::from(&config).build();
    trace!(?config, "inferred AWS S3 configuration from environment");
    let s3 = aws_sdk_s3::Client::from_conf(config);
    let s3_bucket_name = args.s3_bucket_name;
    let app = attune::server::new(
        attune::server::ServerState {
            db,
            s3,
            s3_bucket_name,
        },
        args.default_api_token,
    )
    .await;

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
