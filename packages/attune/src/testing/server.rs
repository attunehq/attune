use aws_config::BehaviorVersion;
use axum_test::TestServer;
use reqwest::Url;
use sha2::{Digest as _, Sha256};
use uuid::{ContextV7, Timestamp};

use crate::api::TenantID;

/// A test server for Attune, and all its parts for manual validation/testing.
pub struct AttuneTestServer {
    /// Raw access to the database.
    pub db: sqlx::PgPool,

    /// The test server.
    ///
    /// This is configured to listen to an open port on the system.
    /// - You can use e.g. `get` or `post` methods to test as a client directly.
    /// - You can use `server_url` to get an external URL for passing to e.g. a
    ///   CLI.
    pub http: TestServer,

    /// The base URL of the test server.
    pub base_url: Url,

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
            // TODO: Migrate all tests to use `create_test_tenant`, and then set
            // this to `None` to remove the footgun.
            Some(http_api_token.clone()),
        )
        .await;

        let http = TestServer::builder()
            .http_transport()
            .build(app)
            .expect("create test server");

        let base_url = http.server_url("/").expect("construct server base url");

        AttuneTestServer {
            http,
            db: config.db,
            s3,
            s3_bucket_name,
            http_api_token,
            base_url,
        }
    }

    /// Allocates a unique tenant for this test run, adding it to the database.
    ///
    /// This is useful for:
    ///
    /// 1. Running multiple tests in the same database, since it prevents tests
    ///    from clobbering each other by reusing the same tenant's repositories.
    ///    However, for sqlx tests, it's not strictly necessary.
    /// 2. Ensuring that tests behave as expected with non-local tenants.
    ///
    /// You should use this for all tests instead of using the default tenant
    /// and API token, because having a different tenant ID per test and test
    /// run ensures that all repositories will have a different S3 prefix. This
    /// will prevent old test runs that upload to S3 from uploading to the same
    /// repository as newer test runs, so you won't accidentally see files from
    /// an old test run when you test for the presence of files in the
    /// repository in S3.
    pub async fn create_test_tenant(&self, test_name: &str) -> (TenantID, String) {
        let run_id = uuid::Uuid::new_v7(Timestamp::now(ContextV7::new()));
        let unique_id = format!("TEST/{test_name}/{run_id}");

        let mut tx = self.db.begin().await.unwrap();
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();

        let tenant = sqlx::query!(
            r#"
            INSERT INTO attune_tenant (
                display_name,
                subdomain,
                created_at,
                updated_at
            )
            VALUES ('TEST_TENANT', $1, NOW(), NOW())
            RETURNING id
            "#,
            &unique_id
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        let api_token = format!("test-api-token-{unique_id}");
        sqlx::query!(
            r#"
            INSERT INTO attune_tenant_api_token (tenant_id, name, token, created_at, updated_at)
            VALUES ($1, 'TEST_TENANT_API_TOKEN', $2, NOW(), NOW())
            ON CONFLICT DO NOTHING;
            "#,
            tenant.id,
            Sha256::digest(&api_token).as_slice().to_vec(),
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();

        (TenantID(tenant.id), api_token)
    }

    /// Creates a unique repository for this test run, and returns the S3 prefix
    /// of the repository. You should _always_ prefer this method over calling
    /// the repository creation endpoint.
    ///
    /// The repository creation endpoint sets repository prefixes based on the
    /// repository's tenant and name. This makes for reproducible prefixes, but
    /// also means that if you create a repository with the same (tenant, name)
    /// multiple times, that repository will be assigned the same S3 prefix.
    ///
    /// This is normally not a problem, because this behavior can never occur in
    /// production. However, we _do_ perform this behavior in tests! And in that
    /// case, having two different test runs have the same S3 prefix can cause
    /// subtle bugs, where a test can succeed on one run and fail on another one
    /// if the test examines repository S3 state, because there were S3 files
    /// left over from the previous run!
    ///
    /// Unfortunately, doing reliable cleanup for S3 prefixes is pretty annoying
    /// (until we build a proper abstraction for S3 and repositories that we can
    /// properly `Drop`), and in either case it's convenient for debugging
    /// reasons to leave around the old test states.
    pub async fn create_repository(&self, tenant_id: TenantID, name: &str) -> String {
        let mut tx = self.db.begin().await.unwrap();
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();

        let run_id = uuid::Uuid::new_v7(Timestamp::now(ContextV7::new()));
        let s3_prefix = format!("{}/{}/{}", tenant_id.0, name, run_id);

        sqlx::query!(
            r#"
            INSERT INTO debian_repository (
                name,
                tenant_id,
                s3_bucket,
                s3_prefix,
                created_at,
                updated_at
            )
            VALUES ($1, $2, $3, $4, NOW(), NOW())
            "#,
            name,
            tenant_id.0,
            self.s3_bucket_name,
            &s3_prefix,
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();

        s3_prefix
    }
}
