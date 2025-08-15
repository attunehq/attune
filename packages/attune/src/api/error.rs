use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bon::Builder;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;

#[derive(Serialize, Deserialize, Builder, Debug, Error)]
#[error("{error} ({status}): {message}")]
pub struct ErrorResponse {
    /// The HTTP status code.
    #[serde(with = "http_serde::status_code")]
    pub status: StatusCode,
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
        // As we encounter other error codes, add them here. For the full list
        // of error codes, see docs:
        // - https://www.postgresql.org/docs/current/errcodes-appendix.html
        if let Some(db) = err.as_database_error()
            && let Some(code) = db.code()
        {
            // 40001: serialization_failure
            //
            // This occurs when concurrent transactions attempt to make
            // non-serializable changes (e.g. by making a transaction that reads
            // data dependently changed by another transaction). One of those
            // transactions will need to be retried.
            //
            // https://www.postgresql.org/docs/current/mvcc-serialization-failure-handling.html
            if code == "40001" {
                return ErrorResponse::builder()
                    .status(StatusCode::CONFLICT)
                    .error("CONCURRENT_INDEX_CHANGE")
                    .message("concurrent index change")
                    .build();
            }
            // 23505: unique_violation
            //
            // This occurs when a unique constraint is violated (i.e. there's a
            // row considered duplicate via UNIQUE index).
            //
            // Note that this can happen DURING transactions! Carefully read
            // this[^1] section of the Postgres docs.
            //
            // Note note: Contrary to the docs, I actually don't think that
            // checking for existing keys guarantees that serialization
            // insertions will not cause unique constraint violations, becuase
            // the checks can occur sequentially between conflicting
            // transactions both inserting to the same key.
            //
            // [^1]: https://www.postgresql.org/docs/current/transaction-iso.html#:~:text=While%20PostgreSQL%27s,to%20insert%20it.
            if code == "23505" {
                // TODO: There's probably a better design here, this feels a
                // little fragile. The problem is that the client retries on
                // 409s, but not very unique violation is actually a retriable
                // error. For example, uploading a package with the same ID
                // tuple but different contents is a unique violation, but it's
                // not retriable (no amount of retries will ever make that
                // work).
                //
                // This works for now because all of the retriable stuff has
                // explicit error paths that proactively check for these unique
                // violations and send explicit 400s rather than relying on this
                // `From` implementation, but this feels too footgun-ey to me.
                return ErrorResponse::builder()
                    .status(StatusCode::CONFLICT)
                    .error("ALREADY_EXISTS")
                    .message("already exists")
                    .build();
            }
        }

        ErrorResponse::builder()
            .status(StatusCode::CONFLICT)
            .error("DATABASE_ERROR")
            .message(format!("internal database error: {err}"))
            .build()
    }
}
