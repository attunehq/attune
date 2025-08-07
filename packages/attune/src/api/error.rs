use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bon::Builder;
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
            status,
            error,
            message,
        }
    }

    pub fn not_found(entity: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: format!("{}_NOT_FOUND", entity.to_uppercase()),
            message: format!("{} not found", entity),
        }
    }
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response<Body> {
        let body = serde_json::to_string(&self).unwrap();
        (self.status, body).into_response()
    }
}
