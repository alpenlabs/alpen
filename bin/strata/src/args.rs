//! CLI argument parsing and environment variable handling.

use std::path::PathBuf;

use argh::FromArgs;

use crate::errors::*;

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub(crate) struct EnvArgs {
    // TODO: relevant items that will be populated from env vars
}

impl EnvArgs {
    pub(crate) fn from_env() -> Self {
        // Here we load particular env vars that should probably override the config.
        Self {}
    }

    /// Get strings of overrides gathered from env.
    pub(crate) fn get_overrides(&self) -> Vec<String> {
        // TODO: add stuffs as necessary
        Vec::new()
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

    /// Rpc host that the client will listen to.
    #[argh(option, description = "rpc host")]
    pub rpc_host: Option<String>,

    /// Rpc port that the client will listen to.
    #[argh(option, description = "rpc port")]
    pub rpc_port: Option<u16>,

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

        Ok(overrides)
    }
}
