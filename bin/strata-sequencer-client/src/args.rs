use std::{env, path::PathBuf};

use argh::FromArgs;

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
        Self {
            otlp_url: env::var("STRATA_OTLP_URL").ok(),
            log_dir: env::var("STRATA_LOG_DIR").ok().map(PathBuf::from),
            log_file_prefix: env::var("STRATA_LOG_FILE_PREFIX").ok(),
            service_label: env::var("STRATA_SVC_LABEL").ok(),
        }
    }

    /// Get file logging configuration if log directory is set.
    /// Uses "strata-sequencer" as default prefix if STRATA_LOG_FILE_PREFIX is not set.
    pub(crate) fn get_file_logging_config(&self) -> Option<strata_common::logging::FileLoggingConfig> {
        self.log_dir.as_ref().map(|dir| {
            let prefix = self
                .log_file_prefix
                .as_deref()
                .unwrap_or("strata-sequencer")
                .to_string();
            strata_common::logging::FileLoggingConfig::new(dir.clone(), prefix)
        })
    }
}

#[derive(Debug, Clone, FromArgs)]
#[argh(description = "Alpen Strata sequencer")]
pub(crate) struct Args {
    #[argh(option, short = 'k', description = "path to sequencer root key")]
    pub sequencer_key: Option<PathBuf>,

    #[argh(option, short = 'h', description = "JSON-RPC host")]
    pub rpc_host: Option<String>,

    #[argh(option, short = 'r', description = "JSON-RPC port")]
    pub rpc_port: Option<u16>,

    #[argh(option, short = 'i', description = "poll interval for duties in ms")]
    pub duty_poll_interval: Option<u64>,

    #[argh(
        option,
        short = 'l',
        description = "evm gas limit per epoch (optional)"
    )]
    pub epoch_gas_limit: Option<u64>,
}
