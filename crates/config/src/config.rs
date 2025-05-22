use std::path::PathBuf;

use bitcoin::Network;
use serde::{Deserialize, Serialize};

use crate::btcio::BtcioConfig;

/// Default value for `rpc_port` in [`ClientConfig`].
const DEFAULT_RPC_PORT: u16 = 8542;

/// Default value for `p2p_port` in [`ClientConfig`].
const DEFAULT_P2P_PORT: u16 = 8543;

/// Default value for `datadir` in [`ClientConfig`].
const DEFAULT_DATADIR: &str = "strata-data";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(Default))]
pub struct ClientConfig {
    /// Addr that the client rpc will listen to.
    pub rpc_host: String,

    /// Port that the client rpc will listen to.
    #[serde(default = "default_rpc_port")]
    pub rpc_port: u16,

    /// P2P port that the client will listen to.
    /// NOTE: This is not used at the moment since we don't actually have p2p.
    #[serde(default = "default_p2p_port")]
    pub p2p_port: u16,

    /// Endpoint that the client will use for syncing blocks. In this case sequencer's rpc
    /// endpoint.
    pub sync_endpoint: Option<String>,

    /// How many l2 blocks to fetch at once while syncing.
    pub l2_blocks_fetch_limit: u64,

    /// The data directory where database contents reside.
    #[serde(default = "default_datadir")]
    pub datadir: PathBuf,

    /// For optimistic transactions, how many times to retry if a write fails.
    pub db_retry_count: u16,

    /// If sequencer tasks should run or not. Default to false.
    #[serde(default)]
    pub is_sequencer: bool,

    /// If client should start in checkpoint sync mode or not. Default to false.
    #[serde(default)]
    pub checkpoint_sync: bool,
}

fn default_p2p_port() -> u16 {
    DEFAULT_P2P_PORT
}

fn default_rpc_port() -> u16 {
    DEFAULT_RPC_PORT
}

fn default_datadir() -> PathBuf {
    DEFAULT_DATADIR.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub l1_follow_distance: u64,
    pub client_checkpoint_interval: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoindConfig {
    pub rpc_url: String,
    pub rpc_user: String,
    pub rpc_password: String,
    pub network: Network,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_interval: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RethELConfig {
    pub rpc_url: String,
    pub secret: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecConfig {
    pub reth: RethELConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub bitcoind: BitcoindConfig,
    pub btcio: BtcioConfig,
    pub sync: SyncConfig,
    pub exec: ExecConfig,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_config_load() {
        let config_string_sequencer = r#"
            [bitcoind]
            rpc_url = "http://localhost:18332"
            rpc_user = "alpen"
            rpc_password = "alpen"
            network = "regtest"

            [client]
            rpc_host = "0.0.0.0"
            rpc_port = 8432
            l2_blocks_fetch_limit = 1000
            sync_endpoint = "9.9.9.9:8432"
            datadir = "/path/to/data/directory"
            sequencer_bitcoin_address = "some_addr"
            sequencer_key = "/path/to/sequencer_key"
            seq_pubkey = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            db_retry_count = 5

            [sync]
            l1_follow_distance = 6
            client_poll_dur_ms = 200
            client_checkpoint_interval = 10

            [exec.reth]
            rpc_url = "http://localhost:8551"
            secret = "1234567890abcdef"

            [btcio.reader]
            client_poll_dur_ms = 200

            [btcio.writer]
            write_poll_dur_ms = 200
            fee_policy = "smart"
            reveal_amount = 100
            bundle_interval_ms = 1000

            [btcio.broadcaster]
            poll_interval_ms = 1000
        "#;

        let config = toml::from_str::<Config>(config_string_sequencer);
        assert!(
            config.is_ok(),
            "should be able to load sequencer TOML config but got: {:?}",
            config.err()
        );

        let config_string_fullnode = r#"
            [bitcoind]
            rpc_url = "http://localhost:18332"
            rpc_user = "alpen"
            rpc_password = "alpen"
            network = "regtest"

            [client]
            rpc_host = "0.0.0.0"
            rpc_port = 8432
            l2_blocks_fetch_limit = 1000
            datadir = "/path/to/data/directory"
            sequencer_bitcoin_address = "some_addr"
            sync_endpoint = "9.9.9.9:8432"
            seq_pubkey = "123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0"
            db_retry_count = 5

            [sync]
            l1_follow_distance = 6
            client_poll_dur_ms = 200
            client_checkpoint_interval = 10

            [btcio.reader]
            client_poll_dur_ms = 200

            [btcio.writer]
            write_poll_dur_ms = 200
            fee_policy = "smart"
            reveal_amount = 100
            bundle_interval_ms = 1000

            [btcio.broadcaster]
            poll_interval_ms = 1000

            [exec.reth]
            rpc_url = "http://localhost:8551"
            secret = "1234567890abcdef"

            [relayer]
            refresh_interval = 10
            stale_duration = 120
            relay_misc = true
        "#;

        let config = toml::from_str::<Config>(config_string_fullnode);
        assert!(
            config.is_ok(),
            "should be able to load full-node TOML config but got: {:?}",
            config.err()
        );
    }
}
