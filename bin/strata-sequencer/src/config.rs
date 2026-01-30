use std::path::PathBuf;

use serde::Deserialize;

use crate::args::Args;

const DEFAULT_DUTY_POLL_INTERVAL: u64 = 1000;

/// Logging configuration for the sequencer client.
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct LoggingConfig {
    /// Service label to append to the service name (e.g., "prod", "dev").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_label: Option<String>,

    /// OpenTelemetry OTLP endpoint URL for distributed tracing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otlp_url: Option<String>,

    /// Directory path for file-based logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,

    /// Prefix for log file names (defaults to "strata-sequencer" if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file_prefix: Option<String>,

    /// Use JSON format for logs instead of compact format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_format: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub sequencer_key: PathBuf,
    pub rpc_host: String,
    pub rpc_port: u16,
    pub duty_poll_interval: u64,

    /// Logging configuration (optional).
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Config {
    pub(crate) fn from_args(args: &Args) -> Result<Config, String> {
        let args = args.clone();
        Ok(Self {
            sequencer_key: args
                .sequencer_key
                .ok_or_else(|| "args: no --sequencer-key provided".to_string())?,
            rpc_host: args
                .rpc_host
                .ok_or_else(|| "args: no --rpc-host provided".to_string())?,
            rpc_port: args
                .rpc_port
                .ok_or_else(|| "args: no --rpc-port provided".to_string())?,
            duty_poll_interval: args
                .duty_poll_interval
                .unwrap_or(DEFAULT_DUTY_POLL_INTERVAL),
            logging: LoggingConfig::default(),
        })
    }

    pub(crate) fn ws_url(&self) -> String {
        format!("ws://{}:{}", self.rpc_host, self.rpc_port)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::args::Args;

    fn args_with(host: String, port: u16, poll: Option<u64>) -> Args {
        Args {
            sequencer_key: Some("dummy.key".into()),
            rpc_host: Some(host),
            rpc_port: Some(port),
            duty_poll_interval: poll,
        }
    }

    proptest! {
        #[test]
        fn ws_url_uses_host_and_port(host in "[a-z0-9.-]{1,32}", port in any::<u16>()) {
            let args = args_with(host.clone(), port, Some(123));
            let cfg = Config::from_args(&args).expect("valid args");
            prop_assert_eq!(cfg.ws_url(), format!("ws://{}:{}", host, port));
        }

        #[test]
        fn duty_poll_interval_defaults(host in "[a-z0-9.-]{1,32}", port in any::<u16>()) {
            let args = args_with(host, port, None);
            let cfg = Config::from_args(&args).expect("valid args");
            prop_assert_eq!(cfg.duty_poll_interval, DEFAULT_DUTY_POLL_INTERVAL);
        }
    }
}
