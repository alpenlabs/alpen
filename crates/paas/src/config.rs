//! Configuration types for PaaS

use std::{collections::HashMap, hash};

use serde::{Deserialize, Serialize};

/// Main PaaS configuration
///
/// Generic over `Backend` type to support different zkVM backends (SP1, Risc0, Native, etc.).
/// In practice, this is typically instantiated as `ProverServiceConfig<ZkVmBackend>`.
#[derive(Debug, Clone)]
pub struct ProverServiceConfig<Backend: Clone + Eq + hash::Hash> {
    /// Worker configuration
    pub workers: WorkerConfig<Backend>,

    /// Optional retry configuration (None = retries disabled)
    pub retry: Option<RetryConfig>,
}

impl<Backend: Clone + Eq + hash::Hash> ProverServiceConfig<Backend> {
    /// Create a new configuration with worker counts per backend
    ///
    /// By default, retries are disabled. Use the builder's `with_retry_config()`
    /// method to enable retries.
    pub fn new(worker_counts: HashMap<Backend, usize>) -> Self {
        Self {
            workers: WorkerConfig {
                worker_count: worker_counts,
            },
            retry: None,
        }
    }
}

/// Worker pool configuration
///
/// Defines the number of concurrent workers (semaphore capacity) for each backend type.
#[derive(Debug, Clone)]
pub struct WorkerConfig<Backend: Clone + Eq + hash::Hash> {
    /// Number of concurrent tasks per backend (semaphore capacity)
    pub worker_count: HashMap<Backend, usize>,
}

/// Retry policy configuration
///
/// TODO: Reconcile with strata_common::RetryConfig - these should potentially
/// be unified into a single retry configuration type used across the codebase.
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
    use crate::ZkVmBackend;

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

    #[test]
    fn test_retry_config_delay() {
        let config = RetryConfig {
            max_retries: 5,
            base_delay_secs: 1,
            multiplier: 2.0,
            max_delay_secs: 30,
        };

        // Test exponential backoff
        assert_eq!(config.calculate_delay(0), 1);
        assert_eq!(config.calculate_delay(1), 2);
        assert_eq!(config.calculate_delay(2), 4);
        assert_eq!(config.calculate_delay(3), 8);

        // Test max delay cap
        assert_eq!(config.calculate_delay(10), 30);

        // Test should_retry
        assert!(config.should_retry(0));
        assert!(config.should_retry(4));
        assert!(!config.should_retry(5));
    }

    #[test]
    fn test_worker_config_creation() {
        let mut worker_count = HashMap::new();
        worker_count.insert(ZkVmBackend::Native, 4);
        worker_count.insert(ZkVmBackend::SP1, 2);

        let config = WorkerConfig {
            worker_count: worker_count.clone(),
        };

        assert_eq!(config.worker_count.get(&ZkVmBackend::Native), Some(&4));
        assert_eq!(config.worker_count.get(&ZkVmBackend::SP1), Some(&2));
        assert_eq!(config.worker_count.get(&ZkVmBackend::Risc0), None);
    }

    #[test]
    fn test_paas_config_creation() {
        let mut worker_count = HashMap::new();
        worker_count.insert(ZkVmBackend::Native, 2);

        let config = ProverServiceConfig::new(worker_count);

        assert!(config.retry.is_none()); // Retries disabled by default
    }
}
