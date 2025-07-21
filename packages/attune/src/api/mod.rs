pub mod pkg;
pub mod repo;
pub mod sign;

use axum::extract::FromRef;

#[derive(Clone, Debug, FromRef)]
pub struct ServerState {
    pub db: sqlx::PgPool,
    pub s3: aws_sdk_s3::Client,

    pub s3_bucket_name: String,
}
