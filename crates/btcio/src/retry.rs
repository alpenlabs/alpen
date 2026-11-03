//! Retry pacing for btcio services that retain their work between attempts.

use std::time::Duration;

use rand::Rng;
use strata_db_types::DbError;
use tokio::time::Instant;

/// Reports whether an error wraps a recoverable database failure.
pub(crate) fn is_retryable_db_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<DbError>()
            .is_some_and(DbError::is_retryable)
    })
}

/// Gates attempts without sleeping in message processing or blocking shutdown.
pub(crate) struct RetryBackoff {
    retry_at: Option<Instant>,
    delay: Duration,
}

impl RetryBackoff {
    pub(crate) fn new() -> Self {
        Self {
            retry_at: None,
            delay: Duration::from_secs(1),
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.retry_at
            .is_none_or(|retry_at| Instant::now() >= retry_at)
    }

    pub(crate) fn is_retrying(&self) -> bool {
        self.retry_at.is_some()
    }

    /// Schedules a retry and reports whether this is the first failure of an outage.
    pub(crate) fn record_failure(&mut self) -> bool {
        let first_failure = self.retry_at.is_none();

        // Jitter between half and all of the current delay, keeping the 30-second ceiling.
        let delay_ms = self.delay.as_millis() as u64;
        let jittered_ms = rand::thread_rng().gen_range(delay_ms / 2..=delay_ms);
        self.retry_at = Some(Instant::now() + Duration::from_millis(jittered_ms));
        self.delay = (self.delay * 2).min(Duration::from_secs(30));

        first_failure
    }

    /// Resets the delay after success and reports whether an outage ended.
    pub(crate) fn reset(&mut self) -> bool {
        let recovered = self.retry_at.take().is_some();
        self.delay = Duration::from_secs(1);
        recovered
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::advance;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retries_are_delayed_capped_and_reset_after_recovery() {
        let mut backoff = RetryBackoff::new();
        assert!(backoff.is_ready());
        assert!(!backoff.is_retrying());

        for attempt in 0..10 {
            assert_eq!(backoff.record_failure(), attempt == 0);
            assert!(!backoff.is_ready());

            let delay = backoff.retry_at.unwrap() - Instant::now();
            let ceiling = Duration::from_secs((1 << attempt).min(30));
            assert!(delay >= ceiling / 2 && delay <= ceiling);

            advance(delay).await;
            assert!(backoff.is_ready());
            assert!(backoff.is_retrying());
        }

        assert!(backoff.reset());
        assert!(backoff.is_ready());
        assert!(!backoff.is_retrying());
        assert!(!backoff.reset());

        assert!(backoff.record_failure());
        advance(Duration::from_secs(1)).await;
        assert!(backoff.is_ready());
    }
}
