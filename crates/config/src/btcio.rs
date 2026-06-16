use bitcoin::FeeRate;
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

/// Configuration for btcio tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtcioConfig {
    pub reader: ReaderConfig,
    pub writer: WriterConfig,
    pub broadcaster: BroadcasterConfig,
    /// Depth, in L1 blocks, after which an L1 block is considered safe from reorgs.
    ///
    /// Drives finality decisions in the CSM worker, the buried-manifest cutoff in OL
    /// block assembly, and reorg handling in the btcio reader/broadcaster.
    ///
    /// A value of `0` is permitted and means the chain follows the L1 tip with no
    /// reorg buffer (a checkpoint finalizes as soon as its L1 block reaches the tip);
    /// larger values require that many confirmations before finalizing.
    #[serde(default = "default_l1_reorg_safe_depth")]
    pub l1_reorg_safe_depth: u32,
}

impl Default for BtcioConfig {
    fn default() -> Self {
        Self {
            reader: ReaderConfig::default(),
            writer: WriterConfig::default(),
            broadcaster: BroadcasterConfig::default(),
            l1_reorg_safe_depth: default_l1_reorg_safe_depth(),
        }
    }
}

const fn default_l1_reorg_safe_depth() -> u32 {
    6
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

    /// Fixed Bitcoin fee rate in sat/vB.
    #[serde(rename = "fixed")]
    Fixed {
        #[serde(rename = "fixed_fee_rate", with = "fee_rate_sat_vb")]
        fee_rate: FeeRate,
    },
}

/// Converts a sat/vB fee rate into [`FeeRate`].
pub fn fee_rate_from_sat_per_vb(fee_rate_sat_per_vb: f64) -> Result<FeeRate, String> {
    if !fee_rate_sat_per_vb.is_finite() || fee_rate_sat_per_vb <= 0.0 {
        return Err(format!("invalid fee rate: {fee_rate_sat_per_vb}"));
    }

    let scaled_sat_per_kwu = fee_rate_sat_per_vb * 250.0;
    if scaled_sat_per_kwu > u64::MAX as f64 {
        return Err(format!("fee rate overflows: {fee_rate_sat_per_vb}"));
    }

    let rounded_sat_per_kwu = scaled_sat_per_kwu.round();
    let rounding_tolerance = f64::EPSILON * scaled_sat_per_kwu.abs().max(1.0) * 8.0;
    let fee_rate_sat_per_kwu =
        if (scaled_sat_per_kwu - rounded_sat_per_kwu).abs() <= rounding_tolerance {
            rounded_sat_per_kwu
        } else {
            scaled_sat_per_kwu.ceil()
        };

    Ok(FeeRate::from_sat_per_kwu(fee_rate_sat_per_kwu as u64))
}

/// Converts a [`FeeRate`] into sat/vB.
pub fn fee_rate_to_sat_per_vb(fee_rate: FeeRate) -> f64 {
    fee_rate.to_sat_per_kwu() as f64 / 250.0
}

mod fee_rate_sat_vb {
    use super::*;

    pub(super) fn serialize<S>(fee_rate: &FeeRate, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(fee_rate_to_sat_per_vb(*fee_rate))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<FeeRate, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fee_rate_sat_per_vb = f64::deserialize(deserializer)?;
        fee_rate_from_sat_per_vb(fee_rate_sat_per_vb).map_err(DeError::custom)
    }
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn fee_rate_sat_kwu_roundtrips_through_sat_vb(sat_per_kwu in 1_u64..=1_000_000_000_000) {
            let fee_rate = FeeRate::from_sat_per_kwu(sat_per_kwu);
            let sat_per_vb = fee_rate_to_sat_per_vb(fee_rate);
            let roundtripped = fee_rate_from_sat_per_vb(sat_per_vb)
                .expect("roundtripped fee rate should parse");

            prop_assert_eq!(roundtripped, fee_rate);
        }

        #[test]
        fn fee_rate_sat_vb_roundtrip_is_idempotent(sat_per_vb in 0.01_f64..=1_000_000_000.0) {
            prop_assume!(sat_per_vb.is_finite());

            let fee_rate = fee_rate_from_sat_per_vb(sat_per_vb)
                .expect("fee rate should parse");
            let roundtripped = fee_rate_from_sat_per_vb(fee_rate_to_sat_per_vb(fee_rate))
                .expect("roundtripped fee rate should parse");

            prop_assert_eq!(roundtripped, fee_rate);
        }

        #[test]
        fn fee_rate_sat_vb_conversion_rounds_up_to_sat_kwu(sat_per_vb in 0.01_f64..=1_000_000_000.0) {
            prop_assume!(sat_per_vb.is_finite());

            let fee_rate = fee_rate_from_sat_per_vb(sat_per_vb)
                .expect("fee rate should parse");
            let scaled_sat_per_kwu = sat_per_vb * 250.0;
            let rounding_tolerance = f64::EPSILON * scaled_sat_per_kwu.abs().max(1.0) * 8.0;
            let sat_per_kwu = fee_rate.to_sat_per_kwu() as f64;

            prop_assert!(sat_per_kwu + rounding_tolerance >= scaled_sat_per_kwu);
            prop_assert!(sat_per_kwu - scaled_sat_per_kwu <= 1.0);
        }
    }
}
