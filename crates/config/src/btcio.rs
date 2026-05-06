use std::num::{NonZeroU32, NonZeroU64};

use serde::{de::Error as DeError, Deserialize, Serialize};

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
    pub l1_fee_policy_config: L1FeePolicyConfig,
    /// How much amount(in sats) to send to reveal address. Must be above dust amount or else
    /// reveal transaction won't be accepted.
    pub reveal_amount: u64,
    /// How often to bundle write intents.
    pub bundle_interval_ms: u64,
    /// Optional fee bumping policy for writer-published transactions.
    #[serde(default)]
    pub fee_bumping: FeeBumpingConfig,
}

impl WriterConfig {
    /// Returns the configured L1 fee-policy configuration.
    pub fn l1_fee_policy_config(&self) -> &L1FeePolicyConfig {
        &self.l1_fee_policy_config
    }

    /// Returns the configured L1 fee policy.
    pub fn fee_policy(&self) -> &FeePolicy {
        self.l1_fee_policy_config.fee_policy()
    }
}

/// Selects how BTCIO handles fee bumping for writer-published transactions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeeBumpPolicy {
    /// Disables automatic fee bumping.
    #[default]
    Disabled,

    /// Reserved for opt-in replace-by-fee once the runtime replacement service is wired.
    Rbf,
}

/// Configures automatic fee bumping for BTCIO writer transactions.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FeeBumpingConfig {
    /// Selects the fee bumping mechanism.
    pub policy: FeeBumpPolicy,

    /// Number of L1 blocks a published transaction may remain unconfirmed before it is stale.
    pub min_age_blocks: NonZeroU32,

    /// Maximum number of broadcast attempts for one replacement chain.
    pub max_attempts: NonZeroU32,

    /// Minimum multiplicative fee increase, expressed in basis points.
    ///
    /// This value must be at least `10_000` so an RBF replacement never lowers
    /// the active fee rate, which would violate BIP-125 replacement rules.
    pub multiplier_bps: u32,

    /// Minimum additive fee-rate increase over the active attempt.
    pub min_fee_rate_delta_sat_vb: NonZeroU64,

    /// Maximum replacement fee rate the service is allowed to use.
    pub max_fee_rate_sat_vb: NonZeroU64,
}

impl FeeBumpingConfig {
    /// Validates the fee bumping configuration.
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.policy, FeeBumpPolicy::Rbf) {
            return Err(
                "fee_bumping.policy = \"rbf\" is not supported until the fee-bumper runtime is wired"
                    .to_string(),
            );
        }
        if self.multiplier_bps < 10_000 {
            return Err(
                "fee_bumping.multiplier_bps must be at least 10_000 so bumps do not lower fees"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Returns whether fee bumping is enabled.
    pub fn is_enabled(&self) -> bool {
        matches!(self.policy, FeeBumpPolicy::Rbf)
    }
}

#[derive(Deserialize)]
struct FeeBumpingConfigUnchecked {
    #[serde(default)]
    policy: FeeBumpPolicy,
    #[serde(default = "default_fee_bumping_min_age_blocks")]
    min_age_blocks: NonZeroU32,
    #[serde(default = "default_fee_bumping_max_attempts")]
    max_attempts: NonZeroU32,
    #[serde(default = "default_fee_bumping_multiplier_bps")]
    multiplier_bps: u32,
    #[serde(default = "default_fee_bumping_min_fee_rate_delta_sat_vb")]
    min_fee_rate_delta_sat_vb: NonZeroU64,
    #[serde(default = "default_fee_bumping_max_fee_rate_sat_vb")]
    max_fee_rate_sat_vb: NonZeroU64,
}

impl<'de> Deserialize<'de> for FeeBumpingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked = FeeBumpingConfigUnchecked::deserialize(deserializer)?;
        let config = Self {
            policy: unchecked.policy,
            min_age_blocks: unchecked.min_age_blocks,
            max_attempts: unchecked.max_attempts,
            multiplier_bps: unchecked.multiplier_bps,
            min_fee_rate_delta_sat_vb: unchecked.min_fee_rate_delta_sat_vb,
            max_fee_rate_sat_vb: unchecked.max_fee_rate_sat_vb,
        };
        config.validate().map_err(DeError::custom)?;
        Ok(config)
    }
}

/// Reusable configuration for resolving Bitcoin fee rates.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct L1FeePolicyConfig {
    /// How fees are determined while creating L1 transactions.
    #[serde(flatten)]
    pub(crate) fee_policy: FeePolicy,
}

impl L1FeePolicyConfig {
    /// Creates an L1 fee-policy configuration for the provided fee policy.
    pub fn new(fee_policy: FeePolicy) -> Self {
        Self { fee_policy }
    }

    /// Returns how fees are determined while creating L1 transactions.
    pub fn fee_policy(&self) -> &FeePolicy {
        &self.fee_policy
    }
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
        /// Base URL for a mempool.space-compatible fee API.
        mempool_base_url: String,
        /// Confirmation target passed to bitcoind's `estimatesmartfee` when the mempool explorer
        /// is unreachable.
        #[serde(
            default = "default_bitcoind_conf_target",
            rename = "mempool_fallback_conf_target"
        )]
        fallback_conf_target: u16,
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

impl FeePolicy {
    /// Returns the configured mempool explorer base URL, if any.
    pub fn mempool_base_url(&self) -> Option<&str> {
        match self {
            Self::MempoolExplorer {
                mempool_base_url, ..
            } => Some(mempool_base_url.as_str()),
            Self::BitcoinD { .. } | Self::Fixed { .. } => None,
        }
    }
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
            l1_fee_policy_config: L1FeePolicyConfig::default(),
            fee_bumping: FeeBumpingConfig::default(),
        }
    }
}

impl Default for FeeBumpingConfig {
    fn default() -> Self {
        Self {
            policy: FeeBumpPolicy::Disabled,
            min_age_blocks: default_fee_bumping_min_age_blocks(),
            max_attempts: default_fee_bumping_max_attempts(),
            multiplier_bps: default_fee_bumping_multiplier_bps(),
            min_fee_rate_delta_sat_vb: default_fee_bumping_min_fee_rate_delta_sat_vb(),
            max_fee_rate_sat_vb: default_fee_bumping_max_fee_rate_sat_vb(),
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

const fn nonzero_u32(value: u32) -> NonZeroU32 {
    match NonZeroU32::new(value) {
        Some(value) => value,
        None => panic!("default value must be non-zero"),
    }
}

const fn nonzero_u64(value: u64) -> NonZeroU64 {
    match NonZeroU64::new(value) {
        Some(value) => value,
        None => panic!("default value must be non-zero"),
    }
}

const fn default_fee_bumping_min_age_blocks() -> NonZeroU32 {
    nonzero_u32(2)
}

const fn default_fee_bumping_max_attempts() -> NonZeroU32 {
    nonzero_u32(5)
}

const fn default_fee_bumping_multiplier_bps() -> u32 {
    12_500
}

const fn default_fee_bumping_min_fee_rate_delta_sat_vb() -> NonZeroU64 {
    nonzero_u64(1)
}

const fn default_fee_bumping_max_fee_rate_sat_vb() -> NonZeroU64 {
    nonzero_u64(1_000)
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
