use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ErrorResponse {
    /// The HTTP status code.
    #[serde(skip)]
    status: StatusCode,
    /// A short, unique error code.
    pub error: String,
    /// A human-readable error message.
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
