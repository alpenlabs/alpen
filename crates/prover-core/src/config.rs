//! Configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------------
// Default constants
//
// Named so the defaults are documented in one place and referenced from the
// `Default` impls rather than sprinkled as bare literals. Tuned against the SP1
// network gateway behaviour that motivated the retry redesign (see the
// prover-core README): resume-class blips are cheap to re-poll (large budget),
// resubmits re-run the whole proof (small budget), and dependency waits recheck
// on a steady cadence rather than exponential backoff.
// ----------------------------------------------------------------------------

/// Resume-class retry budget: network blips / crash recovery re-poll the same
/// request, so this is generous.
const DEFAULT_MAX_RETRIES: u32 = 15;
/// Base delay for task-level exponential backoff, in seconds.
const DEFAULT_BASE_DELAY_SECS: u64 = 5;
/// Exponential backoff multiplier per attempt.
const DEFAULT_MULTIPLIER: f64 = 1.5;
/// Cap on a single task-level backoff delay, in seconds (1 hour).
const DEFAULT_MAX_DELAY_SECS: u64 = 3600;
/// Backoff jitter as a fraction of the delay (`±20%`).
const DEFAULT_JITTER_FRAC: f64 = 0.2;
/// Resubmit-class retry budget: each resubmit re-runs the whole proof, so this
/// is kept much smaller than [`DEFAULT_MAX_RETRIES`].
const DEFAULT_MAX_RESUBMITS: u32 = 3;
/// Steady recheck cadence for a `Blocked` (dependency-wait) task, in seconds.
const DEFAULT_BLOCKED_RECHECK_SECS: u64 = 10;
/// Safety backstop: promote a task that has stayed `Blocked` across this many
/// rechecks to `PermanentFailure`, so a dependency that never materializes
/// (e.g. a receipt for an abandoned batch) can't recheck — and hang its
/// waiters — forever. At the default 10s cadence this is ~24 hours.
const DEFAULT_MAX_BLOCKED_RECHECKS: u32 = 8_640;

/// In-attempt (local) retry defaults for idempotent backend polls.
const DEFAULT_LOCAL_MAX_ATTEMPTS: u32 = 5;
const DEFAULT_LOCAL_BASE_DELAY_MS: u64 = 500;
const DEFAULT_LOCAL_MAX_DELAY_MS: u64 = 10_000;

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
            max_attempts: DEFAULT_LOCAL_MAX_ATTEMPTS,
            base_delay_ms: DEFAULT_LOCAL_BASE_DELAY_MS,
            max_delay_ms: DEFAULT_LOCAL_MAX_DELAY_MS,
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
    /// Safety backstop for dependency waits: a task that stays `Blocked` across
    /// this many rechecks is promoted to `PermanentFailure`. A dependency that
    /// never materializes (e.g. a receipt for an abandoned batch) would
    /// otherwise recheck forever and hang every waiter, since parking doesn't
    /// notify. Sized as a last resort, not a normal timeout.
    pub max_blocked_rechecks: u32,
    /// In-attempt retry budget for idempotent backend ops (see
    /// [`LocalRetryConfig`]).
    pub local: LocalRetryConfig,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay_secs: DEFAULT_BASE_DELAY_SECS,
            multiplier: DEFAULT_MULTIPLIER,
            max_delay_secs: DEFAULT_MAX_DELAY_SECS,
            jitter_frac: DEFAULT_JITTER_FRAC,
            max_resubmits: DEFAULT_MAX_RESUBMITS,
            blocked_recheck_secs: DEFAULT_BLOCKED_RECHECK_SECS,
            max_blocked_rechecks: DEFAULT_MAX_BLOCKED_RECHECKS,
            local: LocalRetryConfig::default(),
        }
    }
}

impl RetryConfig {
    /// Return a copy with out-of-range fields clamped to safe values.
    ///
    /// All fields are `pub` and may be deserialized from operator config, so a
    /// bad value (e.g. `jitter_frac > 1.0`, which drives the backoff factor
    /// negative and collapses the delay to `0` — retrying on every tick) must
    /// not reach the scheduler. The prover sanitizes any injected config
    /// through this on ingestion. Clamps rather than rejects so a slightly
    /// mistyped config still runs with sane backoff instead of failing launch.
    pub fn sanitized(mut self) -> Self {
        self.jitter_frac = self.jitter_frac.clamp(0.0, 1.0);
        // A multiplier below 1 shrinks the backoff each attempt; keep it >= 1
        // so retries actually back off. NaN (which compares false either way)
        // also falls back to the default.
        if self.multiplier.is_nan() || self.multiplier < 1.0 {
            self.multiplier = DEFAULT_MULTIPLIER;
        }
        // A zero base delay would retry with no backoff at all.
        self.base_delay_secs = self.base_delay_secs.max(1);
        // Keep the cap at or above the base so the delay never shrinks below
        // the base. Overflow of `now_secs() + max_delay_secs` is handled by the
        // scheduler's saturating add, so no artificial upper bound is imposed
        // on an operator's chosen ceiling.
        self.max_delay_secs = self.max_delay_secs.max(self.base_delay_secs);
        self
    }

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
    fn sanitized_clamps_out_of_range_jitter() {
        // jitter_frac > 1.0 would drive the backoff factor negative and collapse
        // the delay to 0 for many seeds — retrying every tick. Clamp to [0, 1].
        let cfg = RetryConfig {
            jitter_frac: 5.0,
            ..RetryConfig::default()
        }
        .sanitized();
        assert_eq!(cfg.jitter_frac, 1.0);

        let cfg = RetryConfig {
            jitter_frac: -3.0,
            ..RetryConfig::default()
        }
        .sanitized();
        assert_eq!(cfg.jitter_frac, 0.0);
    }

    #[test]
    fn sanitized_jitter_never_yields_zero_delay() {
        // The concrete failure the clamp prevents: a bad jitter_frac producing a
        // sub-base (here zero) delay so the task retries with no backoff.
        let raw = RetryConfig {
            jitter_frac: 5.0,
            ..RetryConfig::default()
        };
        let cfg = raw.sanitized();
        let base = cfg.calculate_delay(3);
        for seed in 0..10_000u64 {
            assert!(
                cfg.jittered_delay_secs(3, seed) >= (base as f64 * (1.0 - cfg.jitter_frac)) as u64,
                "sanitized jitter must stay within bounds"
            );
        }
    }

    #[test]
    fn sanitized_fixes_degenerate_backoff_inputs() {
        let cfg = RetryConfig {
            multiplier: 0.5,    // would shrink backoff each attempt
            base_delay_secs: 0, // no backoff at all
            max_delay_secs: 0,  // cap below base
            ..RetryConfig::default()
        }
        .sanitized();
        assert!(cfg.multiplier >= 1.0);
        assert!(cfg.base_delay_secs >= 1);
        assert!(cfg.max_delay_secs >= cfg.base_delay_secs);

        // NaN multiplier falls back to the default rather than propagating.
        let cfg = RetryConfig {
            multiplier: f64::NAN,
            ..RetryConfig::default()
        }
        .sanitized();
        assert_eq!(cfg.multiplier, DEFAULT_MULTIPLIER);
    }

    #[test]
    fn sanitized_preserves_valid_config() {
        let cfg = RetryConfig::default();
        let sanitized = cfg.clone().sanitized();
        assert_eq!(cfg.jitter_frac, sanitized.jitter_frac);
        assert_eq!(cfg.multiplier, sanitized.multiplier);
        assert_eq!(cfg.base_delay_secs, sanitized.base_delay_secs);
        assert_eq!(cfg.max_delay_secs, sanitized.max_delay_secs);
    }

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
