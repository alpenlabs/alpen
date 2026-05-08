//! CLI argument parsing and environment variable handling.

use std::{env, path::PathBuf};

use argh::FromArgs;

use crate::errors::*;

const STRATA_ADMIN_RPC_TOKEN: &str = "STRATA_ADMIN_RPC_TOKEN";

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub(crate) struct EnvArgs {
    admin_rpc_token: Option<String>,
}

impl EnvArgs {
    /// Loads environment variables that should override the config.
    pub(crate) fn from_env() -> Result<Self, InitError> {
        Ok(Self {
            admin_rpc_token: env::var(STRATA_ADMIN_RPC_TOKEN).ok(),
        })
    }

    /// Get strings of overrides gathered from env.
    pub(crate) fn get_overrides(&self) -> Vec<String> {
        self.admin_rpc_token
            .as_ref()
            .map(|token| format!("client.admin_rpc_bearer_token={token}"))
            .into_iter()
            .collect()
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

        Ok(overrides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_args_without_token_generate_no_overrides() {
        let env_args = EnvArgs {
            admin_rpc_token: None,
        };
        assert!(env_args.get_overrides().is_empty());
    }

    #[test]
    fn test_env_args_admin_token_override() {
        let env_args = EnvArgs {
            admin_rpc_token: Some("test-token".to_string()),
        };
        assert_eq!(
            env_args.get_overrides(),
            vec!["client.admin_rpc_bearer_token=test-token"]
        );
    }

    #[test]
    fn test_args_admin_rpc_overrides() {
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
            overrides: Vec::new(),
        };

        assert_eq!(
            args.get_internal_overrides().unwrap(),
            vec![
                "client.admin_rpc_host=127.0.0.2",
                "client.admin_rpc_port=9544"
            ]
        );
    }
}
