pub mod compatibility;
pub mod pkg;
pub mod repo;

use std::{any::Any, time::Duration};

use axum::{
    BoxError, Router,
    error_handling::HandleErrorLayer,
    extract::{DefaultBodyLimit, FromRef, Request},
    handler::Handler,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use http::StatusCode;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tower::ServiceBuilder;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::warn;

use crate::api::ErrorResponse;

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
        .route(
            "/repositories/{repository_name}",
            get(repo::info::handler)
                .put(repo::edit::handler)
                .delete(repo::delete::handler),
        )
        .route(
            "/repositories/{repository_name}/index",
            get(repo::index::generate::handler).post(repo::index::sign::handler),
        )
        .route(
            "/repositories/{repository_name}/distributions",
            get(repo::dist::list::handler).post(repo::dist::create::handler),
        )
        .route(
            "/repositories/{repository_name}/distributions/{distribution_name}",
            put(repo::dist::edit::handler).delete(repo::dist::delete::handler),
        )
        .route(
            "/repositories/{repository_name}/distributions/{distribution_name}/sync",
            get(repo::sync::check::handler).post(repo::sync::resync::handler),
        )
        .route(
            "/packages",
            get(pkg::list::handler).post(pkg::upload::handler.layer(DefaultBodyLimit::disable())),
        )
        .route("/packages/{package_sha256sum}", get(pkg::info::handler));

    // The intention of error handling middleware here is that:
    // - `handle_non_success` handles responses from handlers and axum itself, converting errors to `ErrorResponse`.
    // - `handle_middleware_error` handles errors from the middleware stack, converting them to `ErrorResponse`.
    // - `handle_panic` handles panics, converting them to `ErrorResponse`.
    Router::new()
        .nest("/api/v0", api)
        .layer(axum::middleware::from_fn(handle_non_success))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CatchPanicLayer::custom(handle_panic))
                .layer(HandleErrorLayer::new(handle_middleware_error))
                .timeout(Duration::from_secs(600)),
        )
        .with_state(state)
}

async fn handle_non_success(request: Request, next: Next) -> Response {
    let uri = request.uri().to_string();
    let response = next.run(request).await;
    let status = response.status();
    if status.is_success() || status.is_redirection() || status.is_informational() {
        return response;
    }

    // The intention here is to check if the response body is an `ErrorResponse` and, if so, return it as-is.
    // If not, we convert the body to a string and use that as the error message, so long as it's not empty.
    //
    // Note that the response body should only fail to be read if it's larger than the limit we provide `to_bytes`.
    // Since we're using `usize::MAX` as the limit, this should never happen, but may if the limit is changed.
    let (parts, body) = response.into_parts();
    let body = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(body) if !body.is_empty() => {
            if serde_json::from_slice::<ErrorResponse>(&body).is_ok() {
                return Response::from_parts(parts, axum::body::Body::from(body));
            }

            Some(String::from_utf8_lossy(&body).to_string())
        }
        Ok(_) => None,
        Err(e) => {
            warn!("unable to read response body for {uri}: {e}");
            None
        }
    };

    match status {
        StatusCode::NOT_FOUND => ErrorResponse::new(
            status,
            String::from("HTTP_ROUTE_NOT_FOUND"),
            body.unwrap_or_else(|| format!("not found: {uri}")),
        )
        .into_response(),
        StatusCode::METHOD_NOT_ALLOWED => ErrorResponse::new(
            status,
            String::from("HTTP_METHOD_NOT_ALLOWED"),
            body.unwrap_or_else(|| format!("method not allowed: {uri}")),
        )
        .into_response(),
        status if status.is_client_error() => ErrorResponse::new(
            status,
            String::from("HTTP_CLIENT_ERROR_GENERIC"),
            body.unwrap_or_else(|| format!("client error: {status}")),
        )
        .into_response(),
        _ => ErrorResponse::new(
            status,
            String::from("HTTP_SERVER_ERROR_GENERIC"),
            body.unwrap_or_else(|| format!("server error: {status}")),
        )
        .into_response(),
    }
}

fn handle_panic(err: Box<dyn Any + Send + 'static>) -> Response {
    let details = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::from("unknown panic message")
    };

    ErrorResponse::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        String::from("HTTP_SERVER_ERROR_GENERIC"),
        format!("internal server error (panic): {details}"),
    )
    .into_response()
}

async fn handle_middleware_error(err: BoxError) -> ErrorResponse {
    ErrorResponse::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        String::from("HTTP_SERVER_ERROR_GENERIC"),
        format!("internal server error: {err}"),
    )
}
