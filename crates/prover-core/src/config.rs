//! Configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ProverConfig {
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_secs: u64,
    pub multiplier: f64,
    pub max_delay_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 15,
            base_delay_secs: 5,
            multiplier: 1.5,
            max_delay_secs: 3600,
        }
    }
}

impl RetryConfig {
    pub fn calculate_delay(&self, retry_count: u32) -> u64 {
        let delay = self.base_delay_secs as f64 * self.multiplier.powi(retry_count as i32);
        delay.min(self.max_delay_secs as f64) as u64
    }

    pub fn should_retry(&self, retry_count: u32) -> bool {
        retry_count < self.max_retries
    }
}
