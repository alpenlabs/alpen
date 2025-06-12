use super::Backoff;

/// Configuration for exponential retry backoff.
///
/// This struct defines how delays should increase between retry attempts
/// using a fixed-point multiplier. It avoids floating-point math by
/// expressing the multiplier as a ratio (`multiplier / multiplier_base`).
///
/// # Fields
///
/// - `base_delay_ms`: The initial delay in milliseconds before the first retry.
/// - `multiplier`: The numerator of the backoff multiplier. For example, `150` with
///   `multiplier_base = 100` represents a 1.5× multiplier.
/// - `multiplier_base`: The denominator of the backoff multiplier. Used in conjunction with
///   `multiplier` to scale the delay after each retry.
#[derive(Debug)]
pub struct ExponentialBackoff {
    /// Initial delay before the first retry, in milliseconds.
    base_delay_ms: u64,

    /// Numerator of the backoff multiplier (e.g., `150` for 1.5x).
    multiplier: u64,

    /// Denominator of the backoff multiplier (e.g., `100` for 1.5x).
    multiplier_base: u64,
}

impl ExponentialBackoff {
    pub fn new(base_delay_ms: u64, multiplier: u64, multiplier_base: u64) -> Self {
        assert!(multiplier_base != 0);
        Self {
            base_delay_ms,
            multiplier,
            multiplier_base,
        }
    }

    pub fn new_with_default_multiplier(base_delay_ms: u64) -> Self {
        Self {
            base_delay_ms,
            ..Default::default()
        }
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base_delay_ms: 1500, // 1.5 secs should be a sane default
            multiplier: 15,
            multiplier_base: 10,
        }
    }
}

impl Backoff for ExponentialBackoff {
    fn base_delay_ms(&self) -> u64 {
        self.base_delay_ms
    }

    fn next_delay_ms(&self, curr_delay_ms: u64) -> u64 {
        curr_delay_ms * self.multiplier / self.multiplier_base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_exponential_backoff_zero_multiplier_base() {
        // This should panic because we can't have a zero denominator
        let _ = ExponentialBackoff::new(1000, 20, 0);
    }

    #[test]
    fn test_backoff_trait_implementation() {
        let backoff = ExponentialBackoff::new(1000, 20, 10);

        // Test base_delay_ms() method
        assert_eq!(backoff.base_delay_ms(), 1000);

        // Test next_delay_ms() method with initial value
        let delay1 = backoff.next_delay_ms(1000);
        assert_eq!(delay1, 2000); // 1000 * 20 / 10 = 2000

        // Test next_delay_ms() method with subsequent value
        let delay2 = backoff.next_delay_ms(delay1);
        assert_eq!(delay2, 4000); // 2000 * 20 / 10 = 4000

        // Test with default multipliers
        let backoff2 = ExponentialBackoff::default();
        let delay = backoff2.next_delay_ms(1000);
        assert_eq!(delay, 1500); // 1000 * 15 / 10 = 1500

        // test with new_with_default_multiplier
        let backoff3 = ExponentialBackoff::new_with_default_multiplier(2000);
        assert_eq!(backoff3.base_delay_ms, 2000);
        let delay = backoff3.next_delay_ms(1000);
        assert_eq!(delay, 1500); // 1000 * 15 / 10 = 1500
    }
}
