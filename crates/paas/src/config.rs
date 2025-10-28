//! Configuration types for PaaS

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strata_primitives::proof::ProofZkVm;

/// Configuration for PaaS service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaaSConfig {
    /// Worker configuration for different proving backends
    pub workers: WorkerConfig,

    /// Retry policy configuration
    pub retry: RetryConfig,

    /// Optional features
    #[serde(default)]
    pub features: FeatureConfig,
}

impl Default for PaaSConfig {
    fn default() -> Self {
        Self {
            workers: WorkerConfig::default(),
            retry: RetryConfig::default(),
            features: FeatureConfig::default(),
        }
    }
}

/// Worker pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// Number of workers per backend
    #[serde(default = "default_worker_map")]
    pub worker_count: HashMap<ProofZkVm, usize>,

    /// Polling interval for task processor (milliseconds)
    #[serde(default = "default_polling_interval")]
    pub polling_interval_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_count: default_worker_map(),
            polling_interval_ms: default_polling_interval(),
        }
    }
}

fn default_worker_map() -> HashMap<ProofZkVm, usize> {
    let mut map = HashMap::new();
    #[cfg(feature = "sp1")]
    {
        map.insert(ProofZkVm::SP1, 20);
    }
    #[cfg(not(feature = "sp1"))]
    {
        map.insert(ProofZkVm::Native, 5);
    }
    map
}

fn default_polling_interval() -> u64 {
    1000 // 1 second
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Base delay in seconds (first retry)
    #[serde(default = "default_base_delay")]
    pub base_delay_secs: u64,

    /// Multiplier for each subsequent retry
    #[serde(default = "default_multiplier")]
    pub multiplier: f64,

    /// Maximum delay cap in seconds
    #[serde(default = "default_max_delay")]
    pub max_delay_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            base_delay_secs: default_base_delay(),
            multiplier: default_multiplier(),
            max_delay_secs: default_max_delay(),
        }
    }
}

fn default_max_retries() -> u32 {
    15
}

fn default_base_delay() -> u64 {
    5
}

fn default_multiplier() -> f64 {
    1.5
}

fn default_max_delay() -> u64 {
    3600 // 1 hour
}

/// Optional feature flags
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeatureConfig {
    /// Enable checkpoint runner (for standalone mode)
    #[serde(default)]
    pub enable_checkpoint_runner: bool,
}
