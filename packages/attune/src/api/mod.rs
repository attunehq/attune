use http::StatusCode;
use percent_encoding::{AsciiSet, CONTROLS};

pub mod auth;
pub mod error;

pub use auth::TenantID;
pub use error::ErrorResponse;

// This is taken from reqwest, see: https://docs.rs/url/2.5.4/src/url/parser.rs.html#38
pub const PATH_SEGMENT_PERCENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

/// Translate a PSQL error into the server's canonical error response
/// based on the error code.
pub fn translate_psql_error(error: sqlx::Error) -> ErrorResponse {
    tracing::error!(?error, "sqlx error");
    if let Some(database_error) = error.as_database_error()
        && let Some(code) = database_error.code()
    {
        match &*code {
            "40001" => {
                return ErrorResponse::builder()
                    .status(StatusCode::CONFLICT)
                    .error("CONCURRENT_INDEX_CHANGE")
                    .message("concurrent index change")
                    .build();
            }
            _ => {}
        }
    }

    ErrorResponse::builder()
        .status(StatusCode::CONFLICT)
        .error("DATABASE_ERROR")
        .message(format!("internal database error: {error}"))
        .build()
}
