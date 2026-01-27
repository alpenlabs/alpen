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

/// Default DB retry delay in ms.
const DEFAULT_DB_RETRY_DELAY: u64 = 200;

/// Default maximum transactions per block.
const DEFAULT_MAX_TXS_PER_BLOCK: usize = 1000;

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

    /// Db retry delay in ms.
    #[serde(default = "default_db_retry_delay")]
    pub db_retry_delay_ms: u64,

    /// If sequencer tasks should run or not. Default to false.
    #[serde(default)]
    pub is_sequencer: bool,
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

fn default_db_retry_delay() -> u64 {
    DEFAULT_DB_RETRY_DELAY
}

fn default_max_txs_per_block() -> usize {
    DEFAULT_MAX_TXS_PER_BLOCK
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub l1_follow_distance: u64,
    pub client_checkpoint_interval: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequencerConfig {
    /// Maximum number of transactions to fetch from mempool per block.
    #[serde(default = "default_max_txs_per_block")]
    pub max_txs_per_block: usize,
}

/// Default slots per epoch for epoch sealing.
const DEFAULT_SLOTS_PER_EPOCH: u64 = 64;

fn default_slots_per_epoch() -> u64 {
    DEFAULT_SLOTS_PER_EPOCH
}

/// Configuration for epoch sealing policy.
///
/// Determines when epochs should be sealed (i.e., when to create terminal blocks).
/// Different variants support different sealing strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "policy")]
pub enum EpochSealingConfig {
    /// Seal every N slots.
    FixedSlot {
        #[serde(default = "default_slots_per_epoch")]
        slots_per_epoch: u64,
    },
}

impl Default for EpochSealingConfig {
    fn default() -> Self {
        Self::FixedSlot {
            slots_per_epoch: DEFAULT_SLOTS_PER_EPOCH,
        }
    }
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

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingConfig {
    /// Service label to append to the service name (e.g., "prod", "dev").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_label: Option<String>,

    /// OpenTelemetry OTLP endpoint URL for distributed tracing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otlp_url: Option<String>,

    /// Directory path for file-based logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,

    /// Prefix for log file names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file_prefix: Option<String>,

    /// Use JSON format for logs instead of compact format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_format: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub bitcoind: BitcoindConfig,
    pub btcio: BtcioConfig,
    pub sync: SyncConfig,
    pub exec: ExecConfig,

    /// Sequencer configuration (only required if client.is_sequencer = true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequencer: Option<SequencerConfig>,

    /// Epoch sealing configuration (only required if client.is_sequencer = true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch_sealing: Option<EpochSealingConfig>,

    /// Logging configuration (optional section in TOML).
    #[serde(default)]
    pub logging: LoggingConfig,
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

            [sequencer]
            max_txs_per_block = 1000

            [epoch_sealing]
            policy = "FixedSlot"
            slots_per_epoch = 10
        "#;

        let config = toml::from_str::<Config>(config_string_sequencer);
        assert!(
            config.is_ok(),
            "should be able to load sequencer TOML config but got: {:?}",
            config.err()
        );
        let config = config.unwrap();
        assert!(
            config.sequencer.is_some(),
            "sequencer config should be present for sequencer"
        );

        assert!(
            config.epoch_sealing.is_some(),
            "batch builder config should be present for sequencer"
        );

        match config.epoch_sealing.as_ref().unwrap() {
            EpochSealingConfig::FixedSlot { slots_per_epoch } => {
                assert_eq!(
                    *slots_per_epoch, 10,
                    "parsed slots_per_epoch should match TOML value"
                );
            }
        }

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
        let config = config.unwrap();
        assert!(
            config.sequencer.is_none(),
            "sequencer config should be absent for fullnode"
        );

        assert!(
            config.epoch_sealing.is_none(),
            "batcher config should be absent for fullnode"
        );
    }
}
