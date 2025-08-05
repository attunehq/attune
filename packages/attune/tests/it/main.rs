use std::convert::identity;

use aws_config::BehaviorVersion;
use axum_test::TestServer;

#[sqlx::test(migrator = "attune::MIGRATOR")]
async fn migrations_applied(pool: sqlx::PgPool) {
    let table_exists = sqlx::query!(
        "SELECT EXISTS (
            SELECT FROM information_schema.tables
            WHERE table_schema = 'public'
            AND table_name = 'attune_tenant'
        ) as exists",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to check if attune_tenant table exists");

    assert!(
        table_exists.exists.is_some_and(identity),
        "attune_tenant table should exist after migrations"
    );
}

/// A test server for Attune, and all its parts for manual validation/testing.
pub struct AttuneTestServer {
    /// Raw access to the database.
    pub db: sqlx::PgPool,

    /// The test server.
    ///
    /// This is configured to listen to an open port on the system.
    /// - You can use e.g. `get` or `post` methods to test as a client directly.
    /// - You can use `server_url` to get an external URL for passing to e.g. a CLI.
    pub http: TestServer,

    /// The API token for the test server.
    pub http_api_token: String,

    /// The S3 client for the test server.
    pub s3: aws_sdk_s3::Client,

    /// The name of the S3 bucket for the test server.
    pub s3_bucket_name: String,
}

impl AttuneTestServer {
    /// Create a new test server.
    pub async fn new(db: sqlx::PgPool) -> Self {
        let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
        let config = aws_sdk_s3::config::Builder::from(&config).build();
        let s3 = aws_sdk_s3::Client::from_conf(config);

        let s3_bucket_name = String::from("attune-test-0");
        let http_api_token = String::from("test-api-token");

        let app = attune::server::new(
            attune::server::ServerState {
                db: db.clone(),
                s3: s3.clone(),
                s3_bucket_name: s3_bucket_name.clone(),
            },
            Some(http_api_token.clone()),
        )
        .await;

        let http = TestServer::builder()
            .http_transport()
            .build(app)
            .expect("create test server");

        AttuneTestServer {
            http,
            db,
            s3,
            s3_bucket_name,
            http_api_token,
        }
    }
}
