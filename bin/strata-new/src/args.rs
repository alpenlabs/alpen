use std::path::PathBuf;

use argh::FromArgs;
use toml::value::Table;

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
        description = "datadir path that will contain databases"
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
    /// Get strings of overrides gathered from args.
    pub(crate) fn get_overrides(&self) -> Result<Vec<String>, InitError> {
        let mut overrides = self.overrides.clone();
        overrides.extend_from_slice(&self.get_direct_overrides()?);
        Ok(overrides)
    }

    /// Overrides passed directly as args and not as overrides.
    fn get_direct_overrides(&self) -> Result<Vec<String>, InitError> {
        let mut overrides = Vec::new();
        if self.sequencer {
            overrides.push("client.is_sequencer=true".to_string());
        }
        if let Some(datadir) = &self.datadir {
            let dd = datadir.to_str().ok_or(anyhow::anyhow!(
                "Invalid datadir override path {:?}",
                datadir
            ))?;
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

type Override = (String, toml::Value);

/// Parses an overrides This first splits the string by '=' to get key and value and then splits
/// the key by '.' which is the update path.
pub(crate) fn parse_override(override_str: &str) -> Result<Override, ConfigError> {
    let (key, value_str) = override_str
        .split_once("=")
        .ok_or(ConfigError::InvalidOverride(override_str.to_string()))?;
    Ok((key.to_string(), parse_value(value_str)))
}

/// Apply override to config.
pub(crate) fn apply_override(
    path: &str,
    value: toml::Value,
    table: &mut Table,
) -> Result<(), ConfigError> {
    match path.split_once(".") {
        None => {
            table.insert(path.to_string(), value);
            Ok(())
        }
        Some((key, rest)) => {
            if let Some(t) = table.get_mut(key).and_then(|v| v.as_table_mut()) {
                apply_override(rest, value, t)
            } else if table.contains_key(key) {
                Err(ConfigError::TraverseNonTableAt(key.to_string()))
            } else {
                Err(ConfigError::MissingKey(key.to_string()))
            }
        }
    }
}

/// Parses a string into a toml value. First tries as `i64`, then as `bool` and then defaults to
/// `String`.
fn parse_value(str_value: &str) -> toml::Value {
    str_value
        .parse::<i64>()
        .map(toml::Value::Integer)
        .or_else(|_| str_value.parse::<bool>().map(toml::Value::Boolean))
        .unwrap_or_else(|_| toml::Value::String(str_value.to_string()))
}
