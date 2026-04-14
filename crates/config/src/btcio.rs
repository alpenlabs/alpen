use serde::{Deserialize, Serialize};

/// Configuration for btcio tasks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BtcioConfig {
    pub reader: ReaderConfig,
    pub writer: WriterConfig,
    pub broadcaster: BroadcasterConfig,
}

/// Configuration for btcio reader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReaderConfig {
    /// How often to poll btc client
    pub client_poll_dur_ms: u32,
}

/// Configuration for btcio writer/signer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WriterConfig {
    /// How often to invoke the writer.
    pub write_poll_dur_ms: u64,
    /// How the fees are determined.
    #[serde(flatten)]
    pub l1_fee_policy: L1FeePolicyConfig,
    /// How much amount(in sats) to send to reveal address. Must be above dust amount or else
    /// reveal transaction won't be accepted.
    pub reveal_amount: u64,
    /// How often to bundle write intents.
    pub bundle_interval_ms: u64,
}

impl WriterConfig {
    /// Returns the configured L1 fee policy.
    pub fn fee_policy(&self) -> &FeePolicy {
        &self.l1_fee_policy.fee_policy
    }

    /// Returns the configured mempool explorer base URL, if any.
    pub fn mempool_base_url(&self) -> Option<&str> {
        self.l1_fee_policy.mempool_base_url.as_deref()
    }
}

/// Reusable configuration for resolving Bitcoin fee rates.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct L1FeePolicyConfig {
    /// How fees are determined while creating L1 transactions.
    #[serde(flatten)]
    pub fee_policy: FeePolicy,
    /// Base URL for mempool.space-compatible fee API.
    pub mempool_base_url: Option<String>,
    /// Confirmation target passed to bitcoind's `estimatesmartfee` when the mempool explorer is
    /// unreachable and the policy falls back to bitcoind.
    #[serde(default = "default_bitcoind_conf_target")]
    pub mempool_fallback_conf_target: u16,
}

/// Definition of how fees are determined while creating l1 transactions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "fee_policy")]
pub enum FeePolicy {
    /// Use mempool explorer recommended fees endpoint.
    #[serde(rename = "mempool")]
    MempoolExplorer {
        #[serde(default, rename = "mempool_fee_policy")]
        policy: MempoolExplorerFeePolicy,
    },

    /// Use Bitcoin Core's `estimatesmartfee` and the target confirmation parameter is the provided
    /// value.
    #[serde(rename = "bitcoind")]
    BitcoinD {
        #[serde(
            default = "default_bitcoind_conf_target",
            rename = "bitcoind_conf_target"
        )]
        conf_target: u16,
    },

    /// Fixed fee in sat/vB.
    #[serde(rename = "fixed")]
    Fixed {
        #[serde(rename = "fixed_fee_rate")]
        fee_rate: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MempoolExplorerFeePolicy {
    /// Use the "fastest" fee estimate from mempool explorer.
    #[default]
    Fastest,

    /// Use the "half hour" fee estimate from mempool explorer.
    HalfHour,

    /// Use the "hour" fee estimate from mempool explorer.
    Hour,

    /// Use the "economy" fee estimate from mempool explorer.
    Economy,

    /// Use the "minimum" fee estimate from mempool explorer.
    Minimum,
}

/// Configuration for btcio broadcaster.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BroadcasterConfig {
    /// How often to invoke the broadcaster, in ms.
    pub poll_interval_ms: u64,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            write_poll_dur_ms: 5_000,
            reveal_amount: 1_000,
            bundle_interval_ms: 500,
            l1_fee_policy: L1FeePolicyConfig::default(),
        }
    }
}

impl Default for FeePolicy {
    fn default() -> Self {
        Self::BitcoinD {
            conf_target: default_bitcoind_conf_target(),
        }
    }
}

const fn default_bitcoind_conf_target() -> u16 {
    1
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            client_poll_dur_ms: 200,
        }
    }
}

impl Default for BroadcasterConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 5_000,
        }
    }
}
