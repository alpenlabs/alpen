//! Protocol parameters for alpen-cli
//!
//! This module handles configurable protocol constants that can vary by network.
//! Parameters are loaded from JSON files with a fallback hierarchy.

use std::path::{Path, PathBuf};

use bdk_wallet::bitcoin::Network;
use serde::{Deserialize, Serialize};
use serde_json;
use terrors::OneOf;

use crate::{
    errors::{DisplayableError, DisplayedError},
    settings::PROJ_DIRS,
};

/// Protocol parameters for bridge operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CliProtocolParams {
    /// Bitcoin network (mainnet, testnet, signet, regtest)
    pub network: Network,

    /// Number of blocks after bridge-in confirmation before recovery path can be spent
    pub bridge_recover_delay: u32,

    /// Number of blocks to wait before considering a transaction final
    pub bridge_finality_depth: u32,

    /// Bridge deposit amount in satoshis (includes fee buffer)
    pub bridge_in_amount: u64,

    /// Bridge withdrawal amount in satoshis (exact amount)
    pub bridge_out_amount: u64,

    /// Blocks to wait before cleaning up recovery descriptors
    pub recovery_desc_cleanup_delay: u32,
}

impl CliProtocolParams {
    /// Validate that parameters are sensible
    pub fn validate(&self) -> Result<(), String> {
        if self.bridge_recover_delay == 0 {
            return Err("bridge_recover_delay must be greater than 0".to_string());
        }

        if self.bridge_finality_depth == 0 {
            return Err("bridge_finality_depth must be greater than 0".to_string());
        }

        if self.bridge_out_amount > self.bridge_in_amount {
            return Err(
                "bridge_out_amount must be less than or equal to bridge_in_amount".to_string(),
            );
        }

        Ok(())
    }

    /// Load parameters from a JSON file
    pub fn from_file(
        path: &Path,
    ) -> Result<Self, OneOf<(std::io::Error, serde_json::Error, String)>> {
        let contents = std::fs::read_to_string(path).map_err(OneOf::new)?;
        let params: Self = serde_json::from_str(&contents).map_err(OneOf::new)?;
        params.validate().map_err(OneOf::new)?;
        Ok(params)
    }

    /// Get default parameters
    pub fn default() -> Self {
        Self {
            network: Network::Signet,
            bridge_recover_delay: 1008,      // ~1 week 
            bridge_finality_depth: 6,
            bridge_in_amount: 1_000_001_000, // 10 BTC + 1k sats fee
            bridge_out_amount: 1_000_000_000, // 10 BTC
            recovery_desc_cleanup_delay: 100,
        }
    }

}

/// Load protocol parameters with fallback hierarchy
///
/// Loading order:
/// 1. Explicit path provided (highest priority)
/// 2. Environment variable ALPEN_CLI_PARAMS
/// 3. Default config directory: ~/.config/alpen/params.json
/// 4. Built-in defaults (lowest priority)
pub fn load_protocol_params(
    explicit_path: Option<&Path>,
    _network: Network,
) -> Result<CliProtocolParams, DisplayedError> {
    // 1. Check explicit path first
    if let Some(path) = explicit_path {
        return CliProtocolParams::from_file(path)
            .user_error("Failed to load protocol parameters from specified file");
    }

    // 2. Check environment variable
    if let Ok(env_value) = std::env::var("ALPEN_CLI_PARAMS") {
        if let Some(file_path) = env_value.strip_prefix('@') {
            // It's a file path
            let path = Path::new(file_path);
            return CliProtocolParams::from_file(path)
                .user_error("Failed to load protocol parameters from environment variable file");
        } else {
            // It's inline JSON
            let params: CliProtocolParams = serde_json::from_str(&env_value)
                .user_error("Failed to parse protocol parameters from environment variable")?;
            params
                .validate()
                .user_error("Invalid protocol parameters from environment variable")?;
            return Ok(params);
        }
    }

    // 3. Check default location
    let default_path = default_params_path();
    if default_path.exists() {
        return CliProtocolParams::from_file(&default_path)
            .user_error("Failed to load protocol parameters from default location");
    }

    // 4. Use built-in defaults
    Ok(CliProtocolParams::default())
}

/// Get the default path for params file
pub fn default_params_path() -> PathBuf {
    PROJ_DIRS.config_dir().join("params.json")
}

/// Create a default params file at the default location
pub fn create_default_params_file() -> Result<PathBuf, DisplayedError> {
    let path = default_params_path();

    // Create config directory if it doesn't exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).internal_error("Failed to create config directory")?;
    }

    let params = CliProtocolParams::default();
    let json = serde_json::to_string_pretty(&params)
        .internal_error("Failed to serialize default parameters")?;

    std::fs::write(&path, json).user_error("Failed to write default parameters file")?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_accepts_valid_params() {
        let params = CliProtocolParams::default();
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_validation_rejects_zero_recovery_delay() {
        let mut params = CliProtocolParams::default();
        params.bridge_recover_delay = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_validation_rejects_zero_finality_depth() {
        let mut params = CliProtocolParams::default();
        params.bridge_finality_depth = 0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_validation_rejects_invalid_amounts() {
        let mut params = CliProtocolParams::default();
        params.bridge_out_amount = params.bridge_in_amount + 1;
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let params = CliProtocolParams::default();
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: CliProtocolParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params, deserialized);
    }


    #[test]
    fn test_load_params_returns_defaults() {
        // Clear any env var from previous tests
        std::env::remove_var("ALPEN_CLI_PARAMS");

        // When no file exists and no env var, should return defaults
        let params = load_protocol_params(None, Network::Signet).unwrap();
        assert_eq!(params, CliProtocolParams::default());
    }

    #[test]
    fn test_load_params_from_env_inline() {
        let json = r#"{"network":"signet","bridge_recover_delay":100,"bridge_finality_depth":3,"bridge_in_amount":500000000,"bridge_out_amount":500000000,"recovery_desc_cleanup_delay":50}"#;

        // Set env var
        std::env::set_var("ALPEN_CLI_PARAMS", json);

        let params = load_protocol_params(None, Network::Signet).unwrap();
        assert_eq!(params.bridge_recover_delay, 100);
        assert_eq!(params.bridge_finality_depth, 3);

        // Clean up
        std::env::remove_var("ALPEN_CLI_PARAMS");
    }

    #[test]
    fn test_default_params_path() {
        let path = default_params_path();
        assert!(path.to_string_lossy().contains("params.json"));
        assert!(!path.to_string_lossy().contains("params-"));
    }
}
