//! Configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ProverConfig {
    pub retry: Option<RetryConfig>,
}

/// In-attempt (local) retry budget for idempotent backend ops — the gRPC
/// status/proof polls and similar reads.
///
/// This is the fast, in-process tier: a transient blip (e.g. SP1's
/// "Service was not ready" transport error, which the SDK gives up on quickly)
/// is retried here with short backoff so it never escalates to a full
/// task-level retry (5s tick + pipeline restart). Kept small and bounded; the
/// task-level [`RetryConfig`] is the durable backstop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRetryConfig {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for LocalRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 10_000,
        }
    }
}

impl LocalRetryConfig {
    /// Backoff before the `attempt`-th (1-based) in-attempt retry, capped at
    /// `max_delay_ms`.
    pub fn delay(&self, attempt: u32) -> Duration {
        let ms = self.base_delay_ms as f64 * 1.5_f64.powi(attempt.saturating_sub(1) as i32);
        Duration::from_millis(ms.min(self.max_delay_ms as f64) as u64)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_secs: u64,
    pub multiplier: f64,
    pub max_delay_secs: u64,
    /// Randomized spread applied to each backoff delay, as a fraction in
    /// `[0, 1]`. `0.2` jitters the delay to `±20%`. Jitter de-correlates the
    /// wake-up times of many tasks that failed on the same tick, so they don't
    /// retry in a synchronized storm against a shared backend.
    pub jitter_frac: f64,
    /// Budget for resubmit-class retries (a dead remote request resubmitted
    /// fresh). Kept much smaller than `max_retries` because each resubmit
    /// re-runs the whole proof, whereas resume-class retries only re-poll.
    pub max_resubmits: u32,
    /// Default recheck cadence for a `Blocked` task (waiting on a dependency),
    /// in seconds. A steady poll — not exponential backoff — since blocking is
    /// an expected wait, not a failure. A spec can override per task via
    /// [`InputResolution::Blocked`](crate::InputResolution)'s `recheck_after`.
    pub blocked_recheck_secs: u64,
    /// In-attempt retry budget for idempotent backend ops (see
    /// [`LocalRetryConfig`]).
    pub local: LocalRetryConfig,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 15,
            base_delay_secs: 5,
            multiplier: 1.5,
            max_delay_secs: 3600,
            jitter_frac: 0.2,
            max_resubmits: 3,
            blocked_recheck_secs: 10,
            local: LocalRetryConfig::default(),
        }
    }
}

impl RetryConfig {
    /// Deterministic exponential backoff for `retry_count`, capped at
    /// `max_delay_secs`.
    pub fn calculate_delay(&self, retry_count: u32) -> u64 {
        let delay = self.base_delay_secs as f64 * self.multiplier.powi(retry_count as i32);
        delay.min(self.max_delay_secs as f64) as u64
    }

    /// [`Self::calculate_delay`] with deterministic jitter applied.
    ///
    /// `seed` should vary per task (and ideally per attempt) so that distinct
    /// tasks spread out; callers derive it from the task key and retry count.
    /// The result stays within `[base*(1-jitter_frac), base*(1+jitter_frac)]`,
    /// clamped to `max_delay_secs`.
    pub fn jittered_delay_secs(&self, retry_count: u32, seed: u64) -> u64 {
        let base = self.calculate_delay(retry_count) as f64;
        if self.jitter_frac <= 0.0 {
            return base as u64;
        }
        // Map the seed deterministically into [0, 1).
        let frac = (seed % 10_000) as f64 / 10_000.0;
        let factor = 1.0 - self.jitter_frac + 2.0 * self.jitter_frac * frac;
        (base * factor).clamp(0.0, self.max_delay_secs as f64) as u64
    }

    pub fn should_retry(&self, retry_count: u32) -> bool {
        retry_count < self.max_retries
    }

    pub fn should_resubmit(&self, resubmit_count: u32) -> bool {
        resubmit_count < self.max_resubmits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_stays_within_bounds() {
        let cfg = RetryConfig::default();
        let base = cfg.calculate_delay(3);
        let lo = (base as f64 * (1.0 - cfg.jitter_frac)) as u64;
        let hi = (base as f64 * (1.0 + cfg.jitter_frac)) as u64;
        for seed in 0..10_000u64 {
            let d = cfg.jittered_delay_secs(3, seed);
            assert!(
                d >= lo && d <= hi,
                "delay {d} out of [{lo}, {hi}] for seed {seed}"
            );
        }
    }

    #[test]
    fn different_seeds_spread() {
        let cfg = RetryConfig::default();
        let a = cfg.jittered_delay_secs(5, 1);
        let b = cfg.jittered_delay_secs(5, 7_777);
        assert_ne!(a, b, "distinct seeds should produce distinct delays");
    }

    #[test]
    fn zero_jitter_is_deterministic() {
        let cfg = RetryConfig {
            jitter_frac: 0.0,
            ..RetryConfig::default()
        };
        assert_eq!(
            cfg.jittered_delay_secs(4, 123),
            cfg.calculate_delay(4),
            "zero jitter must equal the base delay"
        );
    }

    #[test]
    fn local_retry_delay_grows_and_caps() {
        let cfg = LocalRetryConfig::default();
        assert!(
            cfg.delay(2) >= cfg.delay(1),
            "later attempts back off at least as long"
        );
        assert!(
            (cfg.delay(100).as_millis() as u64) <= cfg.max_delay_ms,
            "delay never exceeds the cap"
        );
    }
}
