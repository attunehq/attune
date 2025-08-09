use std::time::Duration;

use attune::api::ErrorResponse;

/// Base delay in milliseconds for retry backoff
pub const STATIC_RETRY_DELAY_MS: u64 = 2000;

/// Calculate a retry delay with jitter
pub fn calculate_retry_delay() -> Duration {
    Duration::from_millis(STATIC_RETRY_DELAY_MS + rand::random_range(0..STATIC_RETRY_DELAY_MS))
}

/// Check if an error response should trigger a retry
pub fn should_retry(error: &ErrorResponse) -> bool {
    matches!(
        error.error.as_str(),
        "CONCURRENT_INDEX_CHANGE" | "DETACHED_SIGNATURE_VERIFICATION_FAILED"
    )
}
