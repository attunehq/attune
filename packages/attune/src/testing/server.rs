use aws_config::BehaviorVersion;
use axum_test::TestServer;

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

pub struct AttuneTestServerConfig {
    pub db: sqlx::PgPool,
    pub s3_bucket_name: Option<String>,
    pub http_api_token: Option<String>,
}

impl AttuneTestServer {
    /// Create a new test server.
    pub async fn new(config: AttuneTestServerConfig) -> Self {
        let awsconfig = aws_config::defaults(BehaviorVersion::latest()).load().await;
        let s3config = aws_sdk_s3::config::Builder::from(&awsconfig).build();
        let s3 = aws_sdk_s3::Client::from_conf(s3config);

        let s3_bucket_name = config
            .s3_bucket_name
            .unwrap_or(String::from("attune-dev-0"));
        let http_api_token = config
            .http_api_token
            .unwrap_or(String::from("test-api-token"));

        let app = crate::server::new(
            crate::server::ServerState {
                db: config.db.clone(),
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
            db: config.db,
            s3,
            s3_bucket_name,
            http_api_token,
        }
    }
}
