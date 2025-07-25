pub mod compatibility;
pub mod pkg;
pub mod repo;
pub mod sign;

use axum::{
    Router,
    extract::{DefaultBodyLimit, FromRef},
    handler::Handler,
    routing::{delete, get, post, put},
};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::warn;

#[derive(Clone, Debug, FromRef)]
pub struct ServerState {
    pub db: PgPool,
    pub s3: aws_sdk_s3::Client,

    pub s3_bucket_name: String,
}

pub async fn new(state: ServerState, default_api_token: Option<String>) -> Router {
    // Initialize special single-tenant user.
    sqlx::query!(
        r#"
        INSERT INTO attune_tenant (id, display_name, subdomain, created_at, updated_at)
        VALUES (1, 'LOCAL_TENANT', '', NOW(), NOW())
        ON CONFLICT DO NOTHING;
        "#
    )
    .execute(&state.db)
    .await
    .expect("could not initialize single-tenant user");

    // If $ATTUNE_API_TOKEN is set, initialize the special single-tenant API
    // token.
    match default_api_token {
        Some(api_token) => {
            let mut tx = state
                .db
                .begin()
                .await
                .expect("could not start default user initialization");
            sqlx::query!("DELETE FROM attune_tenant_api_token WHERE tenant_id = 1;")
                .execute(&mut *tx)
                .await
                .expect("could not remove existing single-tenant API token");
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
            .expect("could not initialize single-tenant API token");
            tx.commit()
                .await
                .expect("could not commit default user initialization");
        }
        None => {
            warn!("$ATTUNE_API_TOKEN is not set, skipping single-tenant API token initialization")
        }
    }

    // Configure routes.
    let api = Router::new()
        .route("/compatibility", get(compatibility::handler))
        .route(
            "/repositories",
            get(repo::list::handler).post(repo::create::handler),
        )
        .route("/repositories/{repository_name}", put(repo::edit::handler).delete(repo::delete::handler));
    Router::new()
        .nest("/api/v0", api)
        .layer(TraceLayer::new_for_http())
        // FIXME: Use a custom ResponseForPanic so that the response body is a
        // valid `api::ErrorResponse` on 500, which is what all the CLI parsing
        // logic expects on a non-200 response.
        .layer(CatchPanicLayer::new())
        .with_state(state)
}
