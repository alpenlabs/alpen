//! Configuration types for PaaS

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Main PaaS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaaSConfig<B> {
    /// Worker configuration
    pub workers: WorkerConfig<B>,

    /// Retry configuration
    pub retry: RetryConfig,
}

impl<B: Clone + Eq + std::hash::Hash> PaaSConfig<B> {
    /// Create a new configuration with worker counts per backend
    pub fn new(worker_counts: HashMap<B, usize>) -> Self {
        Self {
            workers: WorkerConfig {
                worker_count: worker_counts,
                polling_interval_ms: 1000,
            },
            retry: RetryConfig::default(),
        }
    }
}

/// Worker pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "B: Serialize + for<'de> Deserialize<'de> + Eq + std::hash::Hash")]
pub struct WorkerConfig<B: Eq + std::hash::Hash> {
    /// Number of workers per backend
    pub worker_count: HashMap<B, usize>,

    /// Polling interval for checking pending tasks (milliseconds)
    pub polling_interval_ms: u64,
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,

    /// Base delay in seconds (first retry)
    pub base_delay_secs: u64,

    /// Multiplier for each subsequent retry (exponential backoff)
    pub multiplier: f64,

    /// Maximum delay cap in seconds
    pub max_delay_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 15,
            base_delay_secs: 5,
            multiplier: 1.5,
            max_delay_secs: 3600, // 1 hour
        }
    }
}

impl RetryConfig {
    /// Calculate the delay for a given retry attempt
    pub fn calculate_delay(&self, retry_count: u32) -> u64 {
        if retry_count == 0 {
            return self.base_delay_secs;
        }

        let delay = self.base_delay_secs as f64 * self.multiplier.powi(retry_count as i32);
        delay.min(self.max_delay_secs as f64) as u64
    }

    /// Check if a task should be retried
    pub fn should_retry(&self, retry_count: u32) -> bool {
        retry_count < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_delay_calculation() {
        let config = RetryConfig::default();

        assert_eq!(config.calculate_delay(0), 5);
        assert_eq!(config.calculate_delay(1), 7); // 5 * 1.5 = 7.5 -> 7
        assert_eq!(config.calculate_delay(2), 11); // 5 * 1.5^2 = 11.25 -> 11
    }

    #[test]
    fn test_retry_max_delay() {
        let config = RetryConfig {
            base_delay_secs: 5,
            multiplier: 2.0,
            max_delay_secs: 100,
            max_retries: 15,
        };

        // Should cap at max_delay_secs
        assert_eq!(config.calculate_delay(10), 100);
    }

    #[test]
    fn test_should_retry() {
        let config = RetryConfig {
            max_retries: 3,
            ..Default::default()
        };

        assert!(config.should_retry(0));
        assert!(config.should_retry(1));
        assert!(config.should_retry(2));
        assert!(!config.should_retry(3));
    }
}
