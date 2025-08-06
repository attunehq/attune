use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bon::Builder;
use percent_encoding::{AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};

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
    pub fn new(status: StatusCode, error: String, message: String) -> Self {
        Self {
            error,
            message,
            status,
        }
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response<Body> {
        let body = serde_json::to_string(&self).unwrap();
        (self.status, body).into_response()
    }
}

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
