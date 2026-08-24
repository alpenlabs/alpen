use std::{
    num::{NonZeroU32, NonZeroU64},
    time::Duration,
};

use bitcoin::{Amount, FeeRate};
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

/// Configuration for btcio tasks.
#[derive(Debug, Clone, Serialize)]
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

#[derive(Deserialize)]
struct BtcioConfigUnchecked {
    reader: ReaderConfig,
    writer: WriterConfig,
    broadcaster: BroadcasterConfig,
    #[serde(default = "default_l1_reorg_safe_depth")]
    l1_reorg_safe_depth: u32,
}

impl<'de> Deserialize<'de> for BtcioConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked = BtcioConfigUnchecked::deserialize(deserializer)?;
        let config = Self {
            reader: unchecked.reader,
            writer: unchecked.writer,
            broadcaster: unchecked.broadcaster,
            l1_reorg_safe_depth: unchecked.l1_reorg_safe_depth,
        };
        config.validate().map_err(DeError::custom)?;
        Ok(config)
    }
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

impl BtcioConfig {
    /// Validates relationships between Bitcoin IO policies.
    pub fn validate(&self) -> Result<(), String> {
        self.writer.fee_bumping.validate()?;
        self.broadcaster.validate()?;

        if self.writer.fee_bumping.max_fee_rate_sat_vb > self.broadcaster.max_fee_rate_sat_vb {
            return Err(
                "btcio.writer.fee_bumping.max_fee_rate_sat_vb must not exceed btcio.broadcaster.max_fee_rate_sat_vb"
                    .to_string(),
            );
        }

        Ok(())
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
    /// Fee bumping parameters for writer-published transactions.
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

/// Configures automatic fee bumping for BTCIO writer transactions.
///
/// Fee bumping is unconditional: every writer-published transaction that stays unconfirmed for
/// longer than [`min_age_blocks`](Self::min_age_blocks) is replaced at a higher fee rate. There is
/// no switch to turn it off, because the writer no longer scales its fee estimates to compensate
/// for a stuck transaction. [`max_attempts`](Self::max_attempts) and
/// [`max_fee_rate_sat_vb`](Self::max_fee_rate_sat_vb) are what bound the escalation.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct FeeBumpingConfig {
    /// Minimum time between replacement passes, in ms.
    ///
    /// The pass runs inside the writer's watcher tick, which is paced for payload processing and
    /// is far faster than this work needs: `write_poll_dur_ms` is commonly 200. Without its own
    /// interval the pass would rescan every tx-node record, and re-resolve the fee estimate,
    /// several times a second.
    pub check_interval_ms: NonZeroU64,

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

    /// Maximum absolute fee headroom funded into each reveal transaction.
    pub max_reveal_fee_headroom_sats: NonZeroU64,
}

impl FeeBumpingConfig {
    /// Returns the minimum time between replacement passes.
    pub fn check_interval(&self) -> Duration {
        Duration::from_millis(self.check_interval_ms.get())
    }

    /// Validates the fee bumping configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_attempts.get() > 64 {
            return Err(
                "fee_bumping.max_attempts must be at most 64 because replacement-chain resolution gives up after MAX_REPLACEMENT_CHAIN_HOPS (64) hops, so a larger value cannot be resolved"
                    .to_string(),
            );
        }

        if self.multiplier_bps < 10_000 {
            return Err(
                "fee_bumping.multiplier_bps must be at least 10_000 so bumps do not lower fees"
                    .to_string(),
            );
        }

        if FeeRate::from_sat_per_vb(self.min_fee_rate_delta_sat_vb.get()).is_none() {
            return Err(
                "fee_bumping.min_fee_rate_delta_sat_vb is too large to represent as a Bitcoin fee rate"
                    .to_string(),
            );
        }

        if FeeRate::from_sat_per_vb(self.max_fee_rate_sat_vb.get()).is_none() {
            return Err(
                "fee_bumping.max_fee_rate_sat_vb is too large to represent as a Bitcoin fee rate"
                    .to_string(),
            );
        }

        if self.max_fee_rate_sat_vb < self.min_fee_rate_delta_sat_vb {
            return Err(
                "fee_bumping.max_fee_rate_sat_vb must be at least min_fee_rate_delta_sat_vb"
                    .to_string(),
            );
        }

        Ok(())
    }

    /// Returns the replacement fee-rate ceiling.
    pub fn max_fee_rate(&self) -> FeeRate {
        FeeRate::from_sat_per_vb(self.max_fee_rate_sat_vb.get())
            .expect("config: max fee rate is bounded by validation")
    }

    /// Returns the minimum additive fee-rate increase over the active attempt.
    pub fn min_fee_rate_delta(&self) -> FeeRate {
        FeeRate::from_sat_per_vb(self.min_fee_rate_delta_sat_vb.get())
            .expect("config: min fee-rate delta is bounded by validation")
    }

    /// Returns the maximum absolute fee headroom funded into each reveal transaction.
    pub fn max_reveal_fee_headroom(&self) -> Amount {
        Amount::from_sat(self.max_reveal_fee_headroom_sats.get())
    }
}

/// Mirror of [`FeeBumpingConfig`] that runs [`FeeBumpingConfig::validate`] after deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeeBumpingConfigUnchecked {
    #[serde(default = "default_fee_bumping_check_interval_ms")]
    check_interval_ms: NonZeroU64,
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
    #[serde(default = "default_fee_bumping_max_reveal_fee_headroom_sats")]
    max_reveal_fee_headroom_sats: NonZeroU64,
}

impl<'de> Deserialize<'de> for FeeBumpingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked = FeeBumpingConfigUnchecked::deserialize(deserializer)?;
        let config = Self {
            check_interval_ms: unchecked.check_interval_ms,
            min_age_blocks: unchecked.min_age_blocks,
            max_attempts: unchecked.max_attempts,
            multiplier_bps: unchecked.multiplier_bps,
            min_fee_rate_delta_sat_vb: unchecked.min_fee_rate_delta_sat_vb,
            max_fee_rate_sat_vb: unchecked.max_fee_rate_sat_vb,
            max_reveal_fee_headroom_sats: unchecked.max_reveal_fee_headroom_sats,
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
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BroadcasterConfig {
    /// How often to invoke the broadcaster, in ms.
    pub poll_interval_ms: u64,

    /// Maximum fee rate Bitcoin Core may accept for any transaction broadcast by the service.
    #[serde(default = "default_broadcaster_max_fee_rate_sat_vb")]
    pub max_fee_rate_sat_vb: NonZeroU64,
}

#[derive(Deserialize)]
struct BroadcasterConfigUnchecked {
    poll_interval_ms: u64,
    #[serde(default = "default_broadcaster_max_fee_rate_sat_vb")]
    max_fee_rate_sat_vb: NonZeroU64,
}

impl<'de> Deserialize<'de> for BroadcasterConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked = BroadcasterConfigUnchecked::deserialize(deserializer)?;
        let config = Self {
            poll_interval_ms: unchecked.poll_interval_ms,
            max_fee_rate_sat_vb: unchecked.max_fee_rate_sat_vb,
        };
        config.validate().map_err(DeError::custom)?;
        Ok(config)
    }
}

impl BroadcasterConfig {
    /// Validates the broadcaster configuration.
    pub fn validate(&self) -> Result<(), String> {
        if FeeRate::from_sat_per_vb(self.max_fee_rate_sat_vb.get()).is_none() {
            return Err(
                "btcio.broadcaster.max_fee_rate_sat_vb is too large to represent as a Bitcoin fee rate"
                    .to_string(),
            );
        }

        Ok(())
    }

    /// Returns the per-transaction broadcast fee-rate ceiling.
    pub fn max_fee_rate(&self) -> FeeRate {
        FeeRate::from_sat_per_vb(self.max_fee_rate_sat_vb.get())
            .expect("config: max fee rate is bounded by validation")
    }
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
            check_interval_ms: default_fee_bumping_check_interval_ms(),
            min_age_blocks: default_fee_bumping_min_age_blocks(),
            max_attempts: default_fee_bumping_max_attempts(),
            multiplier_bps: default_fee_bumping_multiplier_bps(),
            min_fee_rate_delta_sat_vb: default_fee_bumping_min_fee_rate_delta_sat_vb(),
            max_fee_rate_sat_vb: default_fee_bumping_max_fee_rate_sat_vb(),
            max_reveal_fee_headroom_sats: default_fee_bumping_max_reveal_fee_headroom_sats(),
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

const fn default_fee_bumping_check_interval_ms() -> NonZeroU64 {
    nonzero_u64(30_000)
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

const fn default_fee_bumping_max_reveal_fee_headroom_sats() -> NonZeroU64 {
    nonzero_u64(10_000_000)
}

const fn default_broadcaster_max_fee_rate_sat_vb() -> NonZeroU64 {
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
            max_fee_rate_sat_vb: default_broadcaster_max_fee_rate_sat_vb(),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const BTCIO_CONFIG_PREFIX: &str = r#"
        [reader]
        client_poll_dur_ms = 200

        [writer]
        write_poll_dur_ms = 200
        fee_policy = "fixed"
        fixed_fee_rate = 1.0
        reveal_amount = 546
        bundle_interval_ms = 200
    "#;

    #[test]
    fn broadcaster_defaults_max_fee_rate() {
        let config: BroadcasterConfig = toml::from_str("poll_interval_ms = 200")
            .expect("broadcaster config should deserialize");

        assert_eq!(config.max_fee_rate_sat_vb, nonzero_u64(1_000));
        assert_eq!(
            config.max_fee_rate(),
            FeeRate::from_sat_per_vb(1_000).unwrap()
        );
    }

    #[test]
    fn btcio_rejects_fee_bumping_ceiling_above_broadcast_guardrail() {
        let input = format!(
            "{BTCIO_CONFIG_PREFIX}\n[writer.fee_bumping]\nmax_fee_rate_sat_vb = 501\n\n[broadcaster]\npoll_interval_ms = 200\nmax_fee_rate_sat_vb = 500"
        );
        let error = toml::from_str::<BtcioConfig>(&input)
            .expect_err("fee bumping must stay within the broadcast guardrail")
            .to_string();

        assert!(error.contains(
            "btcio.writer.fee_bumping.max_fee_rate_sat_vb must not exceed btcio.broadcaster.max_fee_rate_sat_vb"
        ));
    }

    #[test]
    fn btcio_accepts_fee_bumping_ceiling_at_broadcast_guardrail() {
        let input = format!(
            "{BTCIO_CONFIG_PREFIX}\n[writer.fee_bumping]\nmax_fee_rate_sat_vb = 500\n\n[broadcaster]\npoll_interval_ms = 200\nmax_fee_rate_sat_vb = 500"
        );
        let config = toml::from_str::<BtcioConfig>(&input)
            .expect("matching fee ceilings should deserialize");

        assert_eq!(config.writer.fee_bumping.max_fee_rate_sat_vb.get(), 500);
        assert_eq!(config.broadcaster.max_fee_rate_sat_vb.get(), 500);
    }

    #[test]
    fn broadcaster_rejects_unrepresentable_max_fee_rate() {
        let unrepresentable = u64::MAX / 250 + 1;
        let error = toml::from_str::<BroadcasterConfig>(&format!(
            "poll_interval_ms = 200\nmax_fee_rate_sat_vb = {unrepresentable}"
        ))
        .expect_err("an unrepresentable maximum fee rate must be rejected")
        .to_string();

        assert!(error.contains("max_fee_rate_sat_vb is too large to represent"));
    }

    #[test]
    fn fee_bumping_reveal_headroom_serde_default_and_roundtrip() {
        let defaulted: FeeBumpingConfig = toml::from_str("").expect("defaults should deserialize");
        assert_eq!(
            defaulted.max_reveal_fee_headroom_sats,
            nonzero_u64(10_000_000)
        );

        let configured = FeeBumpingConfig {
            max_reveal_fee_headroom_sats: nonzero_u64(42_000),
            ..FeeBumpingConfig::default()
        };
        let encoded = toml::to_string(&configured).expect("config should serialize");
        let decoded: FeeBumpingConfig =
            toml::from_str(&encoded).expect("serialized config should deserialize");

        assert_eq!(decoded, configured);
        assert_eq!(decoded.max_reveal_fee_headroom(), Amount::from_sat(42_000));
    }

    #[test]
    fn fee_bumping_rejects_more_attempts_than_replacement_resolution_can_follow() {
        let error = toml::from_str::<FeeBumpingConfig>("max_attempts = 65")
            .expect_err("more than 64 attempts must be rejected")
            .to_string();

        assert!(error.contains("MAX_REPLACEMENT_CHAIN_HOPS (64)"));
    }

    #[test]
    fn fee_bumping_rejects_unrepresentable_max_fee_rate() {
        let unrepresentable = u64::MAX / 250 + 1;
        let error =
            toml::from_str::<FeeBumpingConfig>(&format!("max_fee_rate_sat_vb = {unrepresentable}"))
                .expect_err("an unrepresentable maximum fee rate must be rejected")
                .to_string();

        assert!(error.contains("max_fee_rate_sat_vb is too large to represent"));
    }

    #[test]
    fn fee_bumping_rejects_unrepresentable_min_fee_rate_delta() {
        let unrepresentable = u64::MAX / 250 + 1;
        let error = toml::from_str::<FeeBumpingConfig>(&format!(
            "min_fee_rate_delta_sat_vb = {unrepresentable}\nmax_fee_rate_sat_vb = {unrepresentable}"
        ))
        .expect_err("an unrepresentable fee-rate delta must be rejected")
        .to_string();

        assert!(error.contains("min_fee_rate_delta_sat_vb is too large to represent"));
    }

    #[test]
    fn fee_bumping_accepts_largest_representable_fee_rates() {
        let largest_representable = u64::MAX / 250;
        let config = toml::from_str::<FeeBumpingConfig>(&format!(
            "min_fee_rate_delta_sat_vb = {largest_representable}\nmax_fee_rate_sat_vb = {largest_representable}"
        ))
        .expect("the largest representable fee rates must be accepted");

        assert_eq!(
            config.min_fee_rate_delta(),
            FeeRate::from_sat_per_vb(largest_representable).unwrap()
        );
        assert_eq!(config.max_fee_rate(), config.min_fee_rate_delta());
    }

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
