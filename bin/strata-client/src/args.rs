use std::path::PathBuf;

use argh::FromArgs;
use toml::value::Table;

use crate::errors::{ConfigError, InitError};

/// Configs overridable by environment. Mostly for sensitive data.
#[derive(Debug, Clone)]
pub struct EnvArgs {
    // TODO: relevant items that will be populated from env vars
}

impl EnvArgs {
    pub fn from_env() -> Self {
        // Here we load particular env vars that should probably override the config.
        Self {}
    }

    /// Get strings of overrides gathered from env.
    pub fn get_overrides(&self) -> Vec<String> {
        // TODO: add stuffs as necessary
        Vec::new()
    }
}

#[derive(Debug, Clone, FromArgs)]
#[argh(description = "Strata client")]
pub struct Args {
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

    /// Switch to using checkpoint sync over sequencer based sync
    #[argh(switch, description = "run client in checkpoint-sync mode")]
    pub checkpoint_sync: bool,

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
    pub fn get_overrides(&self) -> Result<Vec<String>, InitError> {
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
        if self.checkpoint_sync {
            overrides.push("client.checkpoint_sync=true".to_string());
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
pub fn parse_override(override_str: &str) -> Result<Override, ConfigError> {
    let (key, value_str) = override_str
        .split_once("=")
        .ok_or(ConfigError::InvalidOverride(override_str.to_string()))?;
    Ok((key.to_string(), parse_value(value_str)))
}

/// Apply override to config.
pub fn apply_override(
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

#[cfg(test)]
mod test {

    use bitcoin::Network;
    use strata_config::{
        btcio::BtcioConfig, BitcoindConfig, ClientConfig, Config, ExecConfig, RethELConfig,
        SyncConfig,
    };

    use super::*;

    fn get_config() -> Config {
        Config {
            client: ClientConfig {
                rpc_host: "".to_string(),
                rpc_port: 300,
                p2p_port: 300,
                sync_endpoint: None,
                l2_blocks_fetch_limit: 20,
                datadir: "".into(),
                db_retry_count: 3,
                is_sequencer: false,
                checkpoint_sync: false,
            },
            bitcoind: BitcoindConfig {
                rpc_url: "".to_string(),
                rpc_user: "".to_string(),
                rpc_password: "".to_string(),
                network: bitcoin::Network::Regtest,
                retry_count: None,
                retry_interval: None,
            },
            btcio: BtcioConfig {
                reader: Default::default(),
                writer: Default::default(),
                broadcaster: Default::default(),
            },
            exec: ExecConfig {
                reth: RethELConfig {
                    rpc_url: "".to_string(),
                    secret: "".into(),
                },
            },
            sync: SyncConfig {
                l1_follow_distance: 1,
                client_checkpoint_interval: 2,
            },
        }
    }

    #[test]
    fn test_apply_override() {
        let config = get_config();
        let mut toml = toml::Value::try_from(&config).unwrap();
        let table = toml.as_table_mut().unwrap();
        let args = Args {
            config: "config_path".into(),
            datadir: None,
            sequencer: true,
            checkpoint_sync: false,
            rollup_params: None,
            rpc_host: None,
            rpc_port: None,
            overrides: vec![
                "btcio.reader.client_poll_dur_ms=50".to_string(),
                "sync.l1_follow_distance=30".to_string(),
                "client.rpc_host=rpchost".to_string(),
                "bitcoind.network=signet".to_string(),
                "client.datadir=some/data/dir/".to_string(),
            ],
        };

        let overrides = args
            .get_overrides()
            .unwrap()
            .into_iter()
            .map(|x| parse_override(&x).unwrap());

        for (path, val) in overrides {
            apply_override(&path, val, table).unwrap();
        }

        let new_config: Config = toml.try_into().unwrap();

        assert_eq!(new_config.btcio.reader.client_poll_dur_ms, 50);
        assert_eq!(new_config.sync.l1_follow_distance, 30);
        assert_eq!(&new_config.client.rpc_host, "rpchost");
        assert_eq!(new_config.bitcoind.network, Network::Signet);
        assert!(new_config.client.is_sequencer);
        assert_eq!(
            &new_config.client.datadir.to_string_lossy(),
            "some/data/dir/"
        );
    }
}
