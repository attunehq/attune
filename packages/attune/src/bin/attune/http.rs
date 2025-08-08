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
pub async fn get<T: DeserializeOwned>(
    ctx: &Config,
    path: impl AsRef<str> + std::fmt::Debug,
) -> Result<(Option<T>, StatusCode), ErrorResponse> {
    run_request(async || {
        ctx.client
            .get(ctx.endpoint.join(path.as_ref()).unwrap())
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
pub async fn post<K: DeserializeOwned, T: Serialize + std::fmt::Debug>(
    ctx: &Config,
    path: impl AsRef<str> + std::fmt::Debug,
    data: &T,
) -> Result<(Option<K>, StatusCode), ErrorResponse> {
    run_request(async || {
        ctx.client
            .post(ctx.endpoint.join(path.as_ref()).unwrap())
            .json(data)
            .send()
            .await
    })
    .await
}

/// Convenience method for POST requests with `multipart/form-data`.
///
/// ```no_run
/// # use attune::http::post_multipart;
/// # use attune::config::Config;
/// # let config = Config::builder().api_token("test").endpoint("http://localhost:8080").build();
pub async fn post_multipart<T: DeserializeOwned>(
    ctx: &Config,
    path: impl AsRef<str> + std::fmt::Debug,
    data: impl Fn() -> reqwest::multipart::Form,
) -> Result<(Option<T>, StatusCode), ErrorResponse> {
    run_request(async || {
        ctx.client
            .post(ctx.endpoint.join(path.as_ref()).unwrap())
            .multipart(data())
            .send()
            .await
    })
    .await
}

/// Convenience method for DELETE requests.
///
/// ```no_run
/// # use attune::http::delete;
/// # use attune::config::Config;
/// # let config = Config::builder().api_token("test").endpoint("http://localhost:8080").build();
/// let data = delete(&config, "/api/v0/something").await?;
/// ```
pub async fn delete<T: DeserializeOwned>(
    ctx: &Config,
    path: impl AsRef<str> + std::fmt::Debug,
) -> Result<(Option<T>, StatusCode), ErrorResponse> {
    run_request(async || {
        ctx.client
            .delete(ctx.endpoint.join(path.as_ref()).unwrap())
            .send()
            .await
    })
    .await
}

async fn run_request<T, F, G>(runner: G) -> Result<(Option<T>, StatusCode), ErrorResponse>
where
    T: DeserializeOwned,
    F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    G: FnMut() -> F,
{
    let res = runner
        .retry_exponential(DEFAULT_ATTEMPTS)
        .await
        .expect("Could not reach API server");

    let status = res.status();
    let url = res.url().to_string();
    let body = res.text().await.expect("Could not download response");
    if body.is_empty() {
        return Ok((None, status));
    }
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&body) {
        return Err(error);
    }
    if let Ok(data) = serde_json::from_str::<T>(&body) {
        return Ok((Some(data), status));
    }

    tracing::warn!(?url, ?body, "Unknown response");
    Ok((None, status))
}

/// Extension trait for HTTP responses dropping status codes.
pub trait ResponseDropStatus {
    /// The output type after transformation.
    type Output;

    /// Drop the status code from the response.
    fn drop_status(self) -> Self::Output;
}

/// Extension trait for HTTP responses dropping bodies.
pub trait ResponseDropBody {
    /// The output type after transformation.
    type Output;

    /// Drop the body from the response.
    fn drop_body(self) -> Self::Output;
}

/// Extension trait for requiring HTTP bodies have responses.
pub trait ResponseRequiresBody {
    /// The output type after transformation.
    type Output;

    /// Require that the body was present.
    /// Panics otherwise.
    fn require_body(self) -> Self::Output;
}

impl<T> ResponseRequiresBody for Result<(Option<T>, StatusCode), ErrorResponse> {
    type Output = Result<(T, StatusCode), ErrorResponse>;

    #[track_caller]
    fn require_body(self) -> Self::Output {
        self.map(|(body, status)| {
            let body = body.expect("Response body was empty, but was required");
            (body, status)
        })
    }
}

impl<T> ResponseRequiresBody for (Option<T>, StatusCode) {
    type Output = (T, StatusCode);

    #[track_caller]
    fn require_body(self) -> Self::Output {
        let (body, status) = self;
        let body = body.expect("Response body was empty, but was required");
        (body, status)
    }
}

impl<T> ResponseDropBody for (T, StatusCode) {
    type Output = StatusCode;

    fn drop_body(self) -> Self::Output {
        self.1
    }
}

impl<T> ResponseDropBody for Result<(T, StatusCode), ErrorResponse> {
    type Output = Result<StatusCode, ErrorResponse>;

    fn drop_body(self) -> Self::Output {
        self.map(|(_, status)| status)
    }
}

impl<T> ResponseDropStatus for (T, StatusCode) {
    type Output = T;

    fn drop_status(self) -> Self::Output {
        self.0
    }
}

impl<T> ResponseDropStatus for Result<(T, StatusCode), ErrorResponse> {
    type Output = Result<T, ErrorResponse>;

    fn drop_status(self) -> Self::Output {
        self.map(|(body, _)| body)
    }
}

/// Extension trait to add retry functionality to reqwest operations using backon
pub trait ReqwestRetryable<T> {
    /// Retry with exponential backoff.
    ///
    /// Uses exponential backoff starting with 100ms, multiplying by 2 each time,
    /// with jitter and a maximum delay of 10 seconds.
    async fn retry_exponential(self, attempts: NonZeroUsize) -> Result<T, reqwest::Error>;
}

impl<T, F, G> ReqwestRetryable<T> for G
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
