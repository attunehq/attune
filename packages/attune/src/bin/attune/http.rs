use std::{num::NonZeroUsize, time::Duration};

use attune::api::ErrorResponse;
use backon::{ExponentialBuilder, Retryable};
use http::StatusCode;
use nonzero_ext::nonzero;
use serde::de::DeserializeOwned;

use crate::config::Config;

/// The default number of attempts to retry.
pub const DEFAULT_ATTEMPTS: NonZeroUsize = nonzero!(3usize);

/// Convenience methods for HTTP requests.
pub async fn get<T: DeserializeOwned>(ctx: &Config, path: &str) -> Result<T, ErrorResponse> {
    let req = async || {
        ctx.client
            .get(ctx.endpoint.join(path).unwrap())
            .send()
            .await
    };

    let res = req
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
