use std::pin::Pin;

use aws_sdk_s3::config::BehaviorVersion;
use axum::{
    Router,
    body::Body,
    extract::DefaultBodyLimit,
    handler::Handler,
    http::{Request, Response, StatusCode},
    routing::{get, post},
};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tokio::signal;
use tower_http::{
    auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer},
    trace::TraceLayer,
};
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

    // Initialize upstream dependencies.
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

    // Initialize configuration.
    let secret = std::env::var("ATTUNE_SECRET").expect("ATTUNE_SECRET not set");
    let tenant_mode =
        std::env::var("ATTUNE_TENANT_MODE").map_or(api::TenantMode::Single, |v| match v.as_str() {
            "single" => api::TenantMode::Single,
            "multi" => api::TenantMode::Multi,
            other => panic!("invalid ATTUNE_TENANT_MODE value: {}", other),
        });

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
        .layer(AsyncRequireAuthorizationLayer::new(APIToken {
            secret,
            db: db.clone(),
        }))
        .layer(TraceLayer::new_for_http())
        .with_state(api::ServerState {
            db,
            s3,
            s3_bucket_name,
            tenant_mode,
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

#[derive(Clone)]
struct APIToken {
    db: PgPool,
    secret: String,
}

impl<B> AsyncAuthorizeRequest<B> for APIToken
where
    B: Send + 'static,
{
    type RequestBody = B;
    type ResponseBody = Body;
    type Future =
        Pin<Box<dyn Future<Output = Result<Request<B>, Response<Self::ResponseBody>>> + Send>>;

    // TODO: This should be an extractor instead because that way we can pass
    // authentication information to the handler in a type-safe way.
    fn authorize(&mut self, request: Request<B>) -> Self::Future {
        let secret = self.secret.clone();
        let db = self.db.clone();
        Box::pin(async move {
            let token = request
                .headers()
                .get("Authorization")
                .and_then(|header| header.to_str().ok())
                .and_then(|header| header.strip_prefix("Bearer "));
            let token = match token {
                None => {
                    return Err(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::default())
                        .unwrap());
                }
                Some(token) => token,
            };
            let expected = Sha256::digest(format!("{}{}", &secret, token));
            let actual = sqlx::query!(
                "SELECT id FROM attune_tenant_api_token WHERE token = $1",
                expected.as_slice(),
            )
            .fetch_optional(&db)
            .await
            .expect("could not validate API token");
            match actual {
                Some(_) => Ok(request),
                None => Err(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Body::default())
                    .unwrap()),
            }
        })
    }
}
