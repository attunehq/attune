use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bon::Builder;
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Serialize, Deserialize, Builder, Debug)]
pub struct ErrorResponse {
    /// The HTTP status code.
    #[serde(skip)]
    status: StatusCode,
    /// A short, unique error code.
    #[builder(into)]
    pub error: String,
    /// A human-readable error message.
    #[builder(into)]
    pub message: String,
}

impl ErrorResponse {
    pub fn new<E, M>(status: StatusCode, error: E, message: M) -> Self
    where
        E: Into<String>,
        M: Into<String>,
    {
        Self {
            status,
            error: error.into(),
            message: message.into(),
        }
    }

    pub fn not_found<S>(entity: S) -> Self
    where
        S: AsRef<str>,
    {
        Self {
            status: StatusCode::NOT_FOUND,
            error: format!("{}_NOT_FOUND", entity.as_ref().to_uppercase()),
            message: format!("{} not found", entity.as_ref()),
        }
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response<Body> {
        let body = serde_json::to_string(&self).unwrap();
        (self.status, body).into_response()
    }
}

impl From<sqlx::Error> for ErrorResponse {
    fn from(err: sqlx::Error) -> Self {
        error!(error = ?err, "sqlx error");
        if let Some(db) = err.as_database_error() {
            if let Some(code) = db.code() {
                // As we encounter other error codes, add them here.
                // 40001: https://www.postgresql.org/docs/current/mvcc-serialization-failure-handling.html
                if code == "40001" {
                    return ErrorResponse::builder()
                        .status(StatusCode::CONFLICT)
                        .error("CONCURRENT_INDEX_CHANGE")
                        .message("concurrent index change")
                        .build();
                }
            }
        }

        ErrorResponse::builder()
            .status(StatusCode::CONFLICT)
            .error("DATABASE_ERROR")
            .message(format!("internal database error: {err}"))
            .build()
    }
}
