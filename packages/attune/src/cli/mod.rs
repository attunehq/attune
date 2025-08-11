//! Operational code for the Attune CLI.
//!
//! The intention here is that the CLI binary layer
//! is a thin wrapper around this module,
//! primarily intended to facilitate testing.
//!
//! ## Builds
//!
//! The existence of this module means that the server
//! and the CLI have to be built together,
//! but this was already the case given how we structured
//! the project (using `bin` instead of seperate crates).
//!
//! ## Stability
//!
//! The contents of this module are unstable
//! and exempt from any semver guarantees.
use std::time::Duration;

use crate::server::compatibility::{API_VERSION_HEADER, API_VERSION_HEADER_V0_2_0};
use reqwest::{Client, Url};
use uuid::Uuid;

pub mod apt;

/// Configuration for the Attune CLI.
#[derive(Debug)]
pub struct Config {
    pub client: Client,
    pub endpoint: Url,
}

impl Config {
    pub fn new(api_token: String, endpoint: String) -> Self {
        // Parse server API endpoint.
        let endpoint = Url::parse(&endpoint).expect("Invalid Attune API endpoint");

        // Set up default headers.
        let mut headers = reqwest::header::HeaderMap::new();

        // We send this as a header so that a future server can route requests
        // based on this header (which gives us more optionality for preserving
        // backwards compatibility).
        headers.insert(
            API_VERSION_HEADER,
            API_VERSION_HEADER_V0_2_0.parse().unwrap(),
        );
        // _Invocations_ are distinct from _requests_ because a single CLI
        // invocation may spawn multiple API requests.
        headers.insert(
            "X-Invocation-ID",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        headers.insert(
            "Authorization",
            format!("Bearer {api_token}").parse().unwrap(),
        );

        // Build default client.
        let client = Client::builder().default_headers(headers).build().unwrap();
        Self { client, endpoint }
    }
}

/// Infinitely retry an asynchronous function call.
///
/// - `operation` is the function to call.
/// - `should_retry` evaluates whether the operation should be retried.
/// - `retry_delay` provides the duration to wait before retrying.
///
/// Optionally, you can use [`retry_delay_default`] for default delay timings.
pub async fn retry_infinite<T, E>(
    operation: impl AsyncFn() -> Result<T, E>,
    should_retry: impl Fn(&E) -> bool,
    retry_delay: impl Fn(usize) -> Duration,
) -> Result<T, E> {
    for attempt in 0usize.. {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if should_retry(&e) {
                    tokio::time::sleep(retry_delay(attempt)).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    unreachable!("loop is functionally infinite");
}

/// The default retry delay is a static delay of 2 seconds
/// plus a random jitter of up to 2 seconds.
pub fn retry_delay_default(_: usize) -> Duration {
    const STATIC_RETRY_DELAY_MS: u64 = 2000;
    Duration::from_millis(STATIC_RETRY_DELAY_MS + rand::random_range(0..STATIC_RETRY_DELAY_MS))
}
