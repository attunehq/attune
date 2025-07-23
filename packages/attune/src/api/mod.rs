use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ApiError {
    code: String,
    message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ApiResponse<T> {
    Ok { data: T },
    Error { error: ApiError },
}
