//! CLI argument parsing and environment variable handling.

use std::{env, path::PathBuf};

use argh::FromArgs;
use strata_common::logging::FileLoggingConfig;

use crate::errors::*;

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub(crate) struct EnvArgs {
    /// OpenTelemetry OTLP endpoint URL
    pub otlp_url: Option<String>,
    /// Log directory for file logging
    pub log_dir: Option<PathBuf>,
    /// Log file prefix for file logging
    pub log_file_prefix: Option<String>,
    /// Service label to include in service name
    pub service_label: Option<String>,
}

impl EnvArgs {
    pub(crate) fn from_env() -> Self {
        // Here we load particular env vars that should probably override the config.
        Self {
            otlp_url: env::var("STRATA_OTLP_URL").ok(),
            log_dir: env::var("STRATA_LOG_DIR").ok().map(PathBuf::from),
            log_file_prefix: env::var("STRATA_LOG_FILE_PREFIX").ok(),
            service_label: env::var("STRATA_SVC_LABEL").ok(),
        }
    }

    /// Get file logging configuration if log directory is set.
    /// Uses "alpen" as default prefix if STRATA_LOG_FILE_PREFIX is not set.
    pub(crate) fn get_file_logging_config(&self) -> Option<FileLoggingConfig> {
        self.log_dir.as_ref().map(|dir| {
            let prefix = self
                .log_file_prefix
                .as_deref()
                .unwrap_or("alpen")
                .to_string();
            FileLoggingConfig::new(dir.clone(), prefix)
        })
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
