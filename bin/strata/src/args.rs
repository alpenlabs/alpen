//! CLI argument parsing and environment variable handling.

use std::{env, path::PathBuf};

use argh::FromArgs;
use strata_config::{Config, SecretString};

use crate::errors::*;

const STRATA_ADMIN_RPC_TOKEN: &str = "STRATA_ADMIN_RPC_TOKEN";
const STRATA_SUBMIT_RPC_TOKEN: &str = "STRATA_SUBMIT_RPC_TOKEN";
const DEFAULT_HEALTH_CHECK_HOST: &str = "0.0.0.0";
const DEFAULT_HEALTH_CHECK_PORT: u16 = 8080;

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub(crate) struct EnvArgs {
    admin_rpc_token: Option<String>,
    submit_rpc_token: Option<String>,
}

impl EnvArgs {
    /// Loads environment variables that should override the config.
    pub(crate) fn from_env() -> Result<Self, InitError> {
        Ok(Self {
            admin_rpc_token: env::var(STRATA_ADMIN_RPC_TOKEN).ok(),
            submit_rpc_token: env::var(STRATA_SUBMIT_RPC_TOKEN).ok(),
        })
    }

    /// Applies environment-only overrides directly to the parsed config.
    pub(crate) fn apply_to_config(&self, config: &mut Config) {
        if let Some(token) = &self.admin_rpc_token {
            config.client.admin_rpc_bearer_token = Some(SecretString::from(token.clone()));
        }
        if let Some(token) = &self.submit_rpc_token {
            config.client.submit_rpc_bearer_token = Some(SecretString::from(token.clone()));
        }
    }
}

#[derive(Clone, Debug, FromArgs)]
#[argh(description = "Strata OL client")]
pub(crate) struct Args {
    // Config non-overriding args
    #[argh(option, short = 'c', description = "path to configuration")]
    pub config: PathBuf,

    // Config overriding args
    /// Data directory path that will override the path in the config toml.
    #[argh(
        option,
        short = 'd',
        description = "datadir path used mainly for databases"
    )]
    pub datadir: Option<PathBuf>,

    /// Switch that indicates if the client is running as a sequencer.
    #[argh(switch, description = "is sequencer")]
    pub sequencer: bool,

    /// Rollup params path that will override the params in the config toml.
    #[argh(option, description = "rollup params")]
    pub rollup_params: Option<PathBuf>,

    /// Path to the sequencer runtime config TOML file.
    #[argh(option, description = "sequencer runtime config")]
    pub sequencer_config: Option<PathBuf>,

    /// OL genesis params path (JSON file).
    #[argh(option, description = "OL genesis params")]
    pub ol_params: Option<PathBuf>,

    /// Path to ASM params JSON file.
    #[argh(option, description = "asm params")]
    pub asm_params: Option<PathBuf>,

    /// Rpc host that the client will listen to.
    #[argh(option, description = "rpc host")]
    pub rpc_host: Option<String>,

    /// Rpc port that the client will listen to.
    #[argh(option, description = "rpc port")]
    pub rpc_port: Option<u16>,

    /// Admin RPC host that the client will listen to.
    #[argh(option, description = "admin rpc host")]
    pub admin_rpc_host: Option<String>,

    /// Admin RPC port that the client will listen to.
    #[argh(option, description = "admin rpc port")]
    pub admin_rpc_port: Option<u16>,

    /// Submit RPC host that the client will listen to.
    #[argh(option, description = "submit rpc host")]
    pub submit_rpc_host: Option<String>,

    /// Submit RPC port that the client will listen to.
    #[argh(option, description = "submit rpc port")]
    pub submit_rpc_port: Option<u16>,

    /// Host for the HTTP health check endpoint.
    #[argh(
        option,
        default = "DEFAULT_HEALTH_CHECK_HOST.to_string()",
        description = "health check host"
    )]
    pub health_check_host: String,

    /// Port for the HTTP health check endpoint.
    #[argh(
        option,
        default = "DEFAULT_HEALTH_CHECK_PORT",
        description = "health check port"
    )]
    pub health_check_port: u16,

    /// Other generic overrides to the config toml.
    /// Will be used, for example, as `-o btcio.reader.client_poll_dur_ms=1000 -o exec.reth.rpc_url=http://reth`
    #[argh(option, short = 'o', description = "generic config overrides")]
    pub overrides: Vec<String>,
}

impl Args {
    /// Get strings of overrides gathered from user and internal attributes.
    pub(crate) fn get_all_overrides(&self) -> Result<Vec<String>, InitError> {
        let mut overrides = self.overrides.clone();
        overrides.extend_from_slice(&self.get_internal_overrides()?);
        Ok(overrides)
    }

    /// Overrides passed directly as args attributes.
    fn get_internal_overrides(&self) -> Result<Vec<String>, InitError> {
        let mut overrides = Vec::new();
        if self.sequencer {
            overrides.push("client.is_sequencer=true".to_string());
        }
        if let Some(datadir) = &self.datadir {
            let dd = datadir
                .to_str()
                .ok_or_else(|| InitError::InvalidDatadirPath(datadir.clone()))?;
            overrides.push(format!("client.datadir={dd}"));
        }
        if let Some(rpc_host) = &self.rpc_host {
            overrides.push(format!("client.rpc_host={rpc_host}"));
        }
        if let Some(rpc_port) = &self.rpc_port {
            overrides.push(format!("client.rpc_port={rpc_port}"));
        }
        if let Some(admin_rpc_host) = &self.admin_rpc_host {
            overrides.push(format!("client.admin_rpc_host={admin_rpc_host}"));
        }
        if let Some(admin_rpc_port) = &self.admin_rpc_port {
            overrides.push(format!("client.admin_rpc_port={admin_rpc_port}"));
        }
        if let Some(submit_rpc_host) = &self.submit_rpc_host {
            overrides.push(format!("client.submit_rpc_host={submit_rpc_host}"));
        }
        if let Some(submit_rpc_port) = &self.submit_rpc_port {
            overrides.push(format!("client.submit_rpc_port={submit_rpc_port}"));
        }

        Ok(overrides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        toml::from_str(
            r#"
            [bitcoind]
            rpc_url = "http://localhost:18332"
            rpc_user = "alpen"
            rpc_password = "alpen"
            network = "regtest"

            [client]
            rpc_host = "0.0.0.0"
            rpc_port = 8432
            l2_blocks_fetch_limit = 1_000
            datadir = "/path/to/data/directory"
            db_retry_count = 5

            [sync]
            l1_follow_distance = 6
            client_checkpoint_interval = 10

            [btcio.reader]
            client_poll_dur_ms = 200

            [btcio.writer]
            write_poll_dur_ms = 200
            fee_policy = "mempool"
            mempool_base_url = "https://mempool.space/signet"
            reveal_amount = 100
            bundle_interval_ms = 1_000

            [btcio.broadcaster]
            poll_interval_ms = 1_000

            [exec.reth]
            rpc_url = "http://localhost:8551"
            secret = "jwt.hex"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn test_env_args_without_token_leave_config_unchanged() {
        let env_args = EnvArgs {
            admin_rpc_token: None,
            submit_rpc_token: None,
        };
        let mut config = test_config();

        env_args.apply_to_config(&mut config);
        assert_eq!(config.client.admin_rpc_bearer_token, None);
        assert_eq!(config.client.submit_rpc_bearer_token, None);
    }

    #[test]
    fn test_env_args_admin_token_applies_directly_to_config() {
        let env_args = EnvArgs {
            admin_rpc_token: Some("test-token".to_string()),
            submit_rpc_token: None,
        };
        let mut config = test_config();

        env_args.apply_to_config(&mut config);
        assert_eq!(
            config
                .client
                .admin_rpc_bearer_token
                .as_ref()
                .map(SecretString::expose_secret),
            Some("test-token")
        );
    }

    #[test]
    fn test_env_args_submit_token_applies_directly_to_config() {
        let env_args = EnvArgs {
            admin_rpc_token: None,
            submit_rpc_token: Some("test-submit-token".to_string()),
        };
        let mut config = test_config();

        env_args.apply_to_config(&mut config);
        assert_eq!(
            config
                .client
                .submit_rpc_bearer_token
                .as_ref()
                .map(SecretString::expose_secret),
            Some("test-submit-token")
        );
    }

    #[test]
    fn test_args_rpc_overrides() {
        let args = Args {
            config: PathBuf::from("config.toml"),
            datadir: None,
            sequencer: false,
            rollup_params: None,
            sequencer_config: None,
            ol_params: None,
            asm_params: None,
            rpc_host: None,
            rpc_port: None,
            admin_rpc_host: Some("127.0.0.2".to_string()),
            admin_rpc_port: Some(9544),
            submit_rpc_host: Some("127.0.0.3".to_string()),
            submit_rpc_port: Some(9545),
            health_check_host: DEFAULT_HEALTH_CHECK_HOST.to_string(),
            health_check_port: DEFAULT_HEALTH_CHECK_PORT,
            overrides: Vec::new(),
        };

        assert_eq!(
            args.get_internal_overrides().unwrap(),
            vec![
                "client.admin_rpc_host=127.0.0.2",
                "client.admin_rpc_port=9544",
                "client.submit_rpc_host=127.0.0.3",
                "client.submit_rpc_port=9545"
            ]
        );
    }
}
