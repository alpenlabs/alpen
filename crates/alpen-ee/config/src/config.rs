use std::sync::Arc;

use alloy_primitives::U256;
use alpen_ee_common::{FeeModelConfig, L1FeeRateSource};
use strata_config::{btcio::L1FeePolicyConfig, L1FeeRateSourceConfig, SequencerFeeModelConfig};
use strata_predicate::PredicateKey;

use crate::{defaults::DEFAULT_DB_RETRY_COUNT, AlpenEeParams};

/// Local config that may differ between nodes + params.
#[derive(Debug, Clone)]
pub struct AlpenEeConfig {
    /// Chain specific config.
    params: Arc<AlpenEeParams>,

    /// To verify preconfirmed updates from sequencer.
    sequencer_credrule: PredicateKey,

    /// Connection OL RPC client.
    ol_client_http: String,

    /// Connection EE sequencer client.
    ee_sequencer_http: Option<String>,

    /// Number of retries for db connections
    db_retry_count: u16,

    /// Optional v1 fee-model configuration for quoting and charging.
    fee_model: Option<SequencerFeeModelConfig>,

    /// Optional L1 fee-policy config reused for DA pricing.
    l1_fee_policy: Option<L1FeePolicyConfig>,
}

impl AlpenEeConfig {
    /// Creates a new Alpen EE configuration.
    pub fn new(
        params: AlpenEeParams,
        sequencer_credrule: PredicateKey,
        ol_client_http: String,
        ee_sequencer_http: Option<String>,
        db_retry_count: Option<u16>,
    ) -> Self {
        Self {
            params: Arc::new(params),
            sequencer_credrule,
            ol_client_http,
            ee_sequencer_http,
            db_retry_count: db_retry_count.unwrap_or(DEFAULT_DB_RETRY_COUNT),
            fee_model: None,
            l1_fee_policy: None,
        }
    }

    /// Attaches the fee-model configuration used by quote-time RPC and execution-time charging.
    pub fn with_fee_model_config(
        mut self,
        fee_model: SequencerFeeModelConfig,
        l1_fee_policy: L1FeePolicyConfig,
    ) -> Self {
        self.fee_model = Some(fee_model);
        self.l1_fee_policy = Some(l1_fee_policy);
        self
    }

    /// Returns the chain parameters.
    pub fn params(&self) -> &Arc<AlpenEeParams> {
        &self.params
    }

    /// Returns the sequencer credential rule for signature verification.
    pub fn sequencer_credrule(&self) -> &PredicateKey {
        &self.sequencer_credrule
    }

    /// Returns the OL client HTTP connection string.
    pub fn ol_client_http(&self) -> &str {
        &self.ol_client_http
    }

    /// Returns the EE sequencer HTTP connection string if configured.
    pub fn ee_sequencer_http(&self) -> Option<&str> {
        self.ee_sequencer_http.as_deref()
    }

    /// Returns the number of database retries attempted for any transaction.
    pub fn db_retry_count(&self) -> u16 {
        self.db_retry_count
    }

    /// Returns the resolved fee-model configuration, if one has been attached.
    pub fn fee_model(&self) -> Option<FeeModelConfig> {
        self.fee_model.as_ref().map(|fee_model| FeeModelConfig {
            prover_fee_per_gas_wei: u256_from_u64(fee_model.prover_fee_per_gas_wei),
            da_overhead_multiplier_bps: fee_model.da_overhead_multiplier_bps,
            ol_overhead_wei: u256_from_u64(fee_model.ol_overhead_wei),
            l1_fee_rate_source: match fee_model.l1_fee_rate_source {
                L1FeeRateSourceConfig::BtcioWriter => L1FeeRateSource::BtcioWriter,
            },
        })
    }

    /// Returns the attached L1 fee-policy config, if one has been attached.
    pub fn l1_fee_policy(&self) -> Option<&L1FeePolicyConfig> {
        self.l1_fee_policy.as_ref()
    }
}

fn u256_from_u64(value: u64) -> U256 {
    U256::from_limbs([value, 0, 0, 0])
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::L1FeeRateSource;
    use strata_acct_types::AccountId;
    use strata_config::{
        btcio::{FeePolicy, L1FeePolicyConfig},
        L1FeeRateSourceConfig, SequencerFeeModelConfig,
    };
    use strata_predicate::PredicateKey;

    use super::{u256_from_u64, AlpenEeConfig};
    use crate::AlpenEeParams;

    #[test]
    fn test_fee_model_config_can_be_attached_and_read_back() {
        let params = AlpenEeParams::new(
            AccountId::new([1u8; 32]),
            [2u8; 32].into(),
            [3u8; 32].into(),
            0,
        );

        let config = AlpenEeConfig::new(
            params,
            PredicateKey::always_accept(),
            "http://localhost:8542".to_string(),
            Some("http://localhost:8543".to_string()),
            Some(7),
        )
        .with_fee_model_config(
            SequencerFeeModelConfig {
                prover_fee_per_gas_wei: 15,
                da_overhead_multiplier_bps: 12_500,
                ol_overhead_wei: 42,
                l1_fee_rate_source: L1FeeRateSourceConfig::BtcioWriter,
            },
            L1FeePolicyConfig {
                fee_policy: FeePolicy::BitcoinD { conf_target: 6 },
                mempool_base_url: None,
                ..L1FeePolicyConfig::default()
            },
        );

        let fee_model = config.fee_model().expect("fee model must be attached");
        let l1_fee_policy = config
            .l1_fee_policy()
            .expect("l1 fee policy must be attached");

        assert_eq!(config.db_retry_count(), 7);
        assert_eq!(fee_model.prover_fee_per_gas_wei, u256_from_u64(15));
        assert_eq!(fee_model.da_overhead_multiplier_bps, 12_500);
        assert_eq!(fee_model.ol_overhead_wei, u256_from_u64(42));
        assert_eq!(fee_model.l1_fee_rate_source, L1FeeRateSource::BtcioWriter);
        assert_eq!(
            l1_fee_policy.fee_policy,
            FeePolicy::BitcoinD { conf_target: 6 }
        );
    }
}
