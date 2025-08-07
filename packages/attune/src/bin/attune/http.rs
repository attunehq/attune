use std::{num::NonZeroUsize, time::Duration};

use attune::api::ErrorResponse;
use backon::{ExponentialBuilder, Retryable};
use http::StatusCode;
use nonzero_ext::nonzero;
use serde::{Serialize, de::DeserializeOwned};

use crate::config::Config;

/// The default number of attempts to retry.
pub const DEFAULT_ATTEMPTS: NonZeroUsize = nonzero!(3usize);

/// Convenience method for GET requests.
///
/// Uses exponential backoff with jitter to retry the request.
/// ```no_run
/// # use attune::http::get;
/// # use attune::config::Config;
/// # let config = Config::builder().api_token("test").endpoint("http://localhost:8080").build();
/// # #[derive(Debug, serde::Deserialize)]
/// # struct SomeResponse;
///
/// // Responses with JSON bodies:
/// let data = get::<SomeResponse>(&config, "/api/v0/data").await?;
///
/// // Responses without a body, or where you want to ignore the body:
/// let data = get::<()>(&config, "/api/v0/nothing").await?;
/// ```
#[tracing::instrument]
pub async fn get<T: DeserializeOwned>(ctx: &Config, path: &str) -> Result<T, ErrorResponse> {
    run_request(async || {
        ctx.client
            .get(ctx.endpoint.join(path).unwrap())
            .send()
            .await
    })
    .await
}

/// Convenience method for POST requests.
///
/// ```no_run
/// # use attune::http::get;
/// # use attune::config::Config;
/// # let config = Config::builder().api_token("test").endpoint("http://localhost:8080").build();
/// # let body = serde_json::json!({});
/// # #[derive(Debug, serde::Deserialize)]
/// # struct SomeResponse;
///
/// // Responses with JSON bodies:
/// let data = post::<SomeResponse>(&config, "/api/v0/data", &body).await?;
///
/// // Responses without a body, or where you want to ignore the body:
/// let data = post::<()>(&config, "/api/v0/nothing", &body).await?;
///
/// // Requests with JSON bodies:
/// let data = post::<_>(&config, "/api/v0/sending", &body).await?;
///
/// // Requests without a body, or where you want to ignore the body:
/// let data = post::<_>(&config, "/api/v0/nothing", ()).await?;
/// ```
#[tracing::instrument]
pub async fn post<T: Serialize + std::fmt::Debug, K: DeserializeOwned>(
    ctx: &Config,
    path: &str,
    data: &T,
) -> Result<K, ErrorResponse> {
    run_request(async || {
        ctx.client
            .post(ctx.endpoint.join(path).unwrap())
            .json(data)
            .send()
            .await
    })
    .await
}

async fn run_request<T, F, G>(runner: G) -> Result<T, ErrorResponse>
where
    T: DeserializeOwned,
    F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    G: FnMut() -> F,
{
    let res = runner
        .retry_exponential(DEFAULT_ATTEMPTS)
        .await
        .expect("Could not reach API server");

    let body = res.text().await.expect("Could not download response");
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&body) {
        return Err(error);
    }
    if let Ok(data) = serde_json::from_str::<T>(&body) {
        return Ok(data);
    }

    Err(ErrorResponse::builder()
        .status(StatusCode::NOT_IMPLEMENTED)
        .message(format!("Unknown response: {body}"))
        .error("UNKNOWN_RESPONSE")
        .build())
}

/// Extension trait to add retry functionality to reqwest operations using backon
pub trait RetryableHttp<T> {
    /// Retry with exponential backoff.
    ///
    /// Uses exponential backoff starting with 100ms, multiplying by 2 each time,
    /// with jitter and a maximum delay of 10 seconds.
    async fn retry_exponential(self, attempts: NonZeroUsize) -> Result<T, reqwest::Error>;
}

impl<T, F, G> RetryableHttp<T> for G
where
    F: Future<Output = Result<T, reqwest::Error>>,
    G: FnMut() -> F,
{
    #[tracing::instrument(skip(self))]
    async fn retry_exponential(self, attempts: NonZeroUsize) -> Result<T, reqwest::Error> {
        let strategy = ExponentialBuilder::new()
            .with_max_times(attempts.get())
            .with_max_delay(Duration::from_secs(30))
            .with_min_delay(Duration::from_secs(1))
            .with_jitter();

        self.retry(strategy)
            .sleep(tokio::time::sleep)
            .when(|err| should_retry_reqwest_error(err))
            .notify(|err, delay| {
                let url = err.url().map(|url| url.as_str()).unwrap_or("<unknown url>");
                tracing::warn!(?url, ?err, ?delay, "HTTP request failed, will retry");
            })
            .await
    }
}

/// Determine if a response should trigger a retry
fn should_retry_response(code: StatusCode) -> bool {
    if code.is_server_error() {
        return true;
    }

    match code {
        StatusCode::REQUEST_TIMEOUT => true,
        StatusCode::TOO_MANY_REQUESTS => true,
        _ => false,
    }
}

/// Determine if a reqwest error should trigger a retry
fn should_retry_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error.is_connect()
        || error.is_request()
        || error.status().is_some_and(should_retry_response)
}
