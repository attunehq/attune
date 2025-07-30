use axum::http::StatusCode;
use percent_encoding::percent_decode_str;

use crate::api::ErrorResponse;

pub mod create;
pub mod delete;
pub mod edit;
pub mod list;

fn decode_dist_name(name: &str) -> Result<String, ErrorResponse> {
    // The distribution name in the path is percent-encoded.
    match percent_decode_str(name).decode_utf8() {
        Ok(name) => Ok(name.to_string()),
        Err(err) => Err(ErrorResponse::new(
            StatusCode::BAD_REQUEST,
            "INVALID_DIST_NAME".to_string(),
            format!("Invalid distribution name: could not percent decode: {err}"),
        )),
    }
}