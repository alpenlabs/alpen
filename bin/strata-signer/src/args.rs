//! CLI argument definitions.

use std::{env, path::PathBuf};

use argh::FromArgs;

use crate::config::{SecretString, SignerConfig};

const STRATA_ADMIN_RPC_TOKEN: &str = "STRATA_ADMIN_RPC_TOKEN";

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Clone)]
pub(crate) struct EnvArgs {
    admin_rpc_token: Option<SecretString>,
}

impl EnvArgs {
    /// Loads environment variables that should override the config.
    pub(crate) fn from_env() -> Self {
        Self {
            admin_rpc_token: env::var(STRATA_ADMIN_RPC_TOKEN)
                .ok()
                .and_then(SecretString::new_non_empty),
        }
    }

    /// Applies environment-only overrides directly to the parsed config.
    pub(crate) fn apply_to_config(&self, config: &mut SignerConfig) {
        if let Some(token) = &self.admin_rpc_token {
            config.sequencer_admin_bearer_token = Some(token.clone());
        }
    }
}

#[derive(Debug, FromArgs)]
#[argh(description = "Standalone sequencer signer for Strata")]
pub(crate) struct Args {
    /// path to the TOML configuration file
    #[argh(option, short = 'c')]
    pub(crate) config: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(admin_token: Option<&str>) -> SignerConfig {
        SignerConfig {
            sequencer_key: PathBuf::from("/tmp/sequencer.key"),
            sequencer_admin_endpoint: "ws://127.0.0.1:8434".to_string(),
            sequencer_admin_bearer_token: admin_token.map(str::to_string).map(SecretString::from),
            duty_poll_interval: 1_000,
            health_check_host: "127.0.0.1".to_string(),
            health_check_port: 0,
            logging: Default::default(),
        }
    }

    #[test]
    fn test_env_args_without_token_leave_config_unchanged() {
        let env_args = EnvArgs {
            admin_rpc_token: None,
        };
        let mut config = test_config(None);

        env_args.apply_to_config(&mut config);

        assert!(config.sequencer_admin_bearer_token.is_none());
    }

    #[test]
    fn test_empty_env_token_is_ignored() {
        assert!(SecretString::new_non_empty(String::new()).is_none());
    }

    #[test]
    fn test_env_args_without_token_preserves_config_token() {
        let env_args = EnvArgs {
            admin_rpc_token: None,
        };
        let mut config = test_config(Some("config-token"));

        env_args.apply_to_config(&mut config);

        assert_eq!(
            config
                .sequencer_admin_bearer_token
                .as_ref()
                .map(SecretString::expose_secret),
            Some("config-token")
        );
    }

    #[test]
    fn test_env_args_admin_token_applies_directly_to_config() {
        let env_args = EnvArgs {
            admin_rpc_token: Some(SecretString::from("env-token".to_string())),
        };
        let mut config = test_config(None);

        env_args.apply_to_config(&mut config);

        assert_eq!(
            config
                .sequencer_admin_bearer_token
                .as_ref()
                .map(SecretString::expose_secret),
            Some("env-token")
        );
    }

    #[test]
    fn test_env_args_admin_token_overrides_config_token() {
        let env_args = EnvArgs {
            admin_rpc_token: Some(SecretString::from("env-token".to_string())),
        };
        let mut config = test_config(Some("config-token"));

        env_args.apply_to_config(&mut config);

        assert_eq!(
            config
                .sequencer_admin_bearer_token
                .as_ref()
                .map(SecretString::expose_secret),
            Some("env-token")
        );
    }
}
