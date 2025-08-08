use std::convert::identity;

use aws_config::BehaviorVersion;
use axum_test::TestServer;
use time::OffsetDateTime;

#[sqlx::test(migrator = "attune::testing::MIGRATOR")]
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

#[sqlx::test(migrator = "attune::testing::MIGRATOR")]
async fn component_name_validation(pool: sqlx::PgPool) {
    let server = AttuneTestServer::new(pool).await;

    let create_repo = server
        .http
        .post("/api/v0/repositories")
        .add_header("authorization", format!("Bearer {}", server.http_api_token))
        .json(&serde_json::json!({
            "name": "test-repo",
            "s3_bucket": server.s3_bucket_name,
            "s3_prefix": "test"
        }))
        .await;
    assert!(
        create_repo.status_code().is_success(),
        "Repository creation failed with status: {}",
        create_repo.status_code()
    );

    let invalid_components = [
        "comp with spaces",
        "comp@special",
        "comp#hash",
        "comp.dot",
        "comp/slash",
        "",
    ];
    for invalid_component in invalid_components {
        let sign_request = attune::server::repo::index::sign::SignIndexRequest {
            change: attune::server::repo::index::PackageChange {
                repository: String::from("test-repo"),
                distribution: String::from("stable"),
                component: String::from(invalid_component),
                action: attune::server::repo::index::PackageChangeAction::Add {
                    package_sha256sum: String::from("dummy-sha256sum"),
                },
            },
            release_ts: OffsetDateTime::now_utc(),
            clearsigned: String::from("dummy-clearsigned"),
            detachsigned: String::from("dummy-detachsigned"),
            public_key_cert: String::from("dummy-public-key"),
        };

        let response = server
            .http
            .post("/api/v0/repositories/test-repo/index")
            .add_header("authorization", format!("Bearer {}", server.http_api_token))
            .json(&sign_request)
            .await;
        assert_eq!(
            response.status_code(),
            400,
            "Invalid component name should return 400"
        );

        let error: attune::api::ErrorResponse = response.json();
        assert_eq!(error.error, "INVALID_COMPONENT_NAME");
        assert!(
            error
                .message
                .contains("must contain only letters, numbers, underscores, and hyphens")
        );
    }

    // Test valid component names (these should get further before failing due to dummy data)
    let valid_components = [
        "main",
        "contrib",
        "non-free",
        "my_component",
        "comp123",
        "test-component",
    ];
    for valid_component in valid_components {
        let sign_request = attune::server::repo::index::sign::SignIndexRequest {
            change: attune::server::repo::index::PackageChange {
                repository: String::from("test-repo"),
                distribution: String::from("stable"),
                component: String::from(valid_component),
                action: attune::server::repo::index::PackageChangeAction::Add {
                    package_sha256sum: String::from("dummy-sha256sum"),
                },
            },
            release_ts: OffsetDateTime::now_utc(),
            clearsigned: String::from("dummy-clearsigned"),
            detachsigned: String::from("dummy-detachsigned"),
            public_key_cert: String::from("dummy-public-key"),
        };
        let response = server
            .http
            .post("/api/v0/repositories/test-repo/index")
            .add_header("authorization", format!("Bearer {}", server.http_api_token))
            .json(&sign_request)
            .await;

        // These should not fail with INVALID_COMPONENT_NAME (although they will fail due to dummy data)
        let body = response.json::<attune::api::ErrorResponse>();
        assert_ne!(
            body.error, "INVALID_COMPONENT_NAME",
            "Valid component '{valid_component}' should not be rejected with INVALID_COMPONENT_NAME",
        );
    }
}
