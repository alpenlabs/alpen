//! CLI argument parsing and environment variable handling.

use std::{
    env::{self, VarError},
    path::PathBuf,
};

use argh::FromArgs;

use crate::errors::*;

/// Environment variable for overriding the OL block time in milliseconds.
const OL_BLOCK_TIME_MS_ENVVAR: &str = "STRATA_OL_BLOCK_TIME_MS";

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub(crate) struct EnvArgs {
    ol_block_time_ms: Option<u64>,
}

impl EnvArgs {
    /// Loads environment variables that should override the config.
    pub(crate) fn from_env() -> Result<Self, InitError> {
        Ok(Self {
            ol_block_time_ms: read_ol_block_time_ms_envvar()?,
        })
    }

    /// Get strings of overrides gathered from env.
    pub(crate) fn get_overrides(&self) -> Vec<String> {
        self.ol_block_time_ms
            .map(|ol_block_time_ms| vec![format!("sequencer.ol_block_time_ms={ol_block_time_ms}")])
            .unwrap_or_default()
    }
}

fn read_ol_block_time_ms_envvar() -> Result<Option<u64>, InitError> {
    match env::var(OL_BLOCK_TIME_MS_ENVVAR) {
        Ok(value) => parse_ol_block_time_ms(&value).map(Some),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(InitError::InvalidOlBlockTimeMs(format!(
            "environment variable {OL_BLOCK_TIME_MS_ENVVAR} must be valid UTF-8"
        ))),
    }
}

fn parse_ol_block_time_ms(value: &str) -> Result<u64, InitError> {
    let ol_block_time_ms = value.parse::<u64>().map_err(|_| {
        InitError::InvalidOlBlockTimeMs(format!(
            "environment variable {OL_BLOCK_TIME_MS_ENVVAR} must be a positive integer in milliseconds"
        ))
    })?;

    if ol_block_time_ms == 0 {
        return Err(InitError::InvalidOlBlockTimeMs(format!(
            "environment variable {OL_BLOCK_TIME_MS_ENVVAR} must be greater than 0"
        )));
    }

    Ok(ol_block_time_ms)
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

    #[cfg(feature = "sequencer")]
    /// Path to sequencer root key (required with `--sequencer`).
    #[argh(option, short = 'k', description = "path to sequencer root key")]
    pub sequencer_key: Option<PathBuf>,

    #[cfg(feature = "sequencer")]
    /// Poll interval for duties in ms.
    #[argh(option, short = 'i', description = "poll interval for duties in ms")]
    pub duty_poll_interval: Option<u64>,

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ol_block_time_ms_accepts_positive_integer() {
        assert_eq!(parse_ol_block_time_ms("5000").unwrap(), 5000);
    }

    #[test]
    fn test_parse_ol_block_time_ms_rejects_zero() {
        let error = parse_ol_block_time_ms("0").unwrap_err().to_string();
        assert!(error.contains("must be greater than 0"));
    }

    #[test]
    fn test_parse_ol_block_time_ms_rejects_non_integer() {
        let error = parse_ol_block_time_ms("five-seconds")
            .unwrap_err()
            .to_string();
        assert!(error.contains("positive integer"));
    }

    #[test]
    fn test_env_args_generates_ol_block_time_override() {
        let env_args = EnvArgs {
            ol_block_time_ms: Some(5000),
        };
        assert_eq!(
            env_args.get_overrides(),
            vec!["sequencer.ol_block_time_ms=5000".to_string()]
        );
    }
}
