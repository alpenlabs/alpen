#[derive(Clone, Copy, Debug)]
pub(crate) struct ExponentialBackoff {
    /// Maximum number of retries.
    max_retries: u64,
    /// Total time in seconds across all retries.
    total_time: u64,
    /// The base for exponential growth.
    base: f64,
}

impl ExponentialBackoff {
    pub(crate) const fn new(max_retries: u64, total_time: u64, base: f64) -> Self {
        Self {
            max_retries,
            total_time,
            base,
        }
    }

    /// Returns the delay in seconds.
    pub(crate) fn get_delay(&self, retry_counter: u64) -> u64 {
        if retry_counter == 0 {
            return 0;
        }

        // Geometric series sum: S_n = (1 - base^n) / (1 - base)
        let sum_of_series = (1.0 - self.base.powf(self.max_retries as f64)) / (1.0 - self.base);
        let base_delay = self.total_time as f64 / sum_of_series;
        let delay = base_delay * self.base.powf((retry_counter - 1) as f64);

        // Convert to whole seconds
        delay.round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::ExponentialBackoff;

    #[test]
    fn test_total_time() {
        let total_expected = 3600;
        let num_retries = 15;
        let retry_strategy = ExponentialBackoff::new(num_retries, total_expected, 1.5);
        let mut total_time = 0;
        for i in 0u64..=num_retries {
            total_time += retry_strategy.get_delay(i);
        }

        assert_eq!(total_time, total_expected);
    }

    #[test]
    fn test_zeroth_delay_is_zero() {
        assert_eq!(ExponentialBackoff::new(2, 100, 1.5).get_delay(0), 0);
    }
}
