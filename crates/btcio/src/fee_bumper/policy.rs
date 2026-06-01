//! Fee bumping policy calculation.

use bitcoin::{Amount, FeeRate};
use strata_config::btcio::FeeBumpingConfig;
use strata_db_types::types::{TerminalError, TxAttempt, TxNodeRecord};
use strata_primitives::L1Height;

/// A concrete fee-bump request for one active transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBumpRequest {
    /// Replacement fee rate.
    pub target_fee_rate: FeeRate,
    /// Attempt number to assign to the replacement.
    pub attempt_no: u32,
}

/// Policy decision for one transaction node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeeBumpDecision {
    /// The transaction is not eligible for replacement yet.
    Wait,
    /// The transaction should be replaced.
    Replace(FeeBumpRequest),
    /// The replacement chain cannot advance further.
    Terminal(TerminalError),
}

/// Evaluates whether an active published transaction should be replaced.
pub fn evaluate_fee_bump(
    config: &FeeBumpingConfig,
    record: &TxNodeRecord,
    active_attempt: &TxAttempt,
    current_l1_tip: L1Height,
    estimate_fee_rate: FeeRate,
    replacement_vsize: usize,
) -> FeeBumpDecision {
    let Some(first_published_height) = active_attempt.first_published_l1_height else {
        return FeeBumpDecision::Wait;
    };

    let age = current_l1_tip.saturating_sub(first_published_height);
    if age < config.min_age_blocks.get() {
        return FeeBumpDecision::Wait;
    }

    if record.attempts.len() >= config.max_attempts.get() as usize {
        return FeeBumpDecision::Terminal(TerminalError::MaxAttemptsReached);
    }

    let Some(active_fee_rate) = fee_rate_from_sat_vb(active_attempt.fee_rate_sat_vb) else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };
    let Some(min_fee_rate_delta) = fee_rate_from_sat_vb(config.min_fee_rate_delta_sat_vb.get())
    else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };
    let Some(max_fee_rate) = fee_rate_from_sat_vb(config.max_fee_rate_sat_vb.get()) else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };

    let active_fee_rate_sat_vb = active_fee_rate.to_sat_per_vb_ceil();
    let Some(additive) = fee_rate_from_sat_vb(
        active_fee_rate_sat_vb.saturating_add(min_fee_rate_delta.to_sat_per_vb_ceil()),
    ) else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };
    let multiplicative_fee_rate_sat_vb = active_fee_rate_sat_vb
        .saturating_mul(config.multiplier_bps as u64)
        .div_ceil(10_000);
    let Some(multiplicative) = fee_rate_from_sat_vb(multiplicative_fee_rate_sat_vb) else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };
    let Some(bip125_min) = bip125_minimum_fee_rate(
        Amount::from_sat(active_attempt.fee_sats),
        min_fee_rate_delta,
        replacement_vsize,
    ) else {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    };

    let target = estimate_fee_rate
        .max(additive)
        .max(multiplicative)
        .max(bip125_min);
    if target > max_fee_rate {
        return FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate);
    }

    FeeBumpDecision::Replace(FeeBumpRequest {
        target_fee_rate: target,
        attempt_no: record.attempts.len() as u32,
    })
}

/// Converts the BIP-125 absolute-fee floor into the replacement's fee rate.
pub fn bip125_minimum_fee_rate(
    active_fee: Amount,
    incremental_relay_fee_rate: FeeRate,
    replacement_vsize: usize,
) -> Option<FeeRate> {
    if replacement_vsize == 0 {
        return Some(FeeRate::ZERO);
    }
    let relay_fee = incremental_relay_fee_rate.fee_vb(replacement_vsize as u64)?;
    let required_fee = active_fee.checked_add(relay_fee)?;
    fee_rate_from_sat_vb(required_fee.to_sat().div_ceil(replacement_vsize as u64))
}

/// Builds a Bitcoin fee-rate value after validating that it fits the type.
pub fn fee_rate_from_sat_vb(fee_rate_sat_vb: u64) -> Option<FeeRate> {
    FeeRate::from_sat_per_vb(fee_rate_sat_vb)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32, NonZeroU64};

    use bitcoin::{absolute::LockTime, transaction::Version, Amount, Transaction};
    use strata_config::btcio::{FeeBumpPolicy, FeeBumpingConfig};
    use strata_db_types::types::{TxAttempt, TxNodeKind, TxNodeRecord};

    use super::*;

    fn config() -> FeeBumpingConfig {
        FeeBumpingConfig {
            policy: FeeBumpPolicy::Rbf,
            min_age_blocks: NonZeroU32::new(2).unwrap(),
            target_inclusion_blocks: NonZeroU16::new(1).unwrap(),
            max_attempts: NonZeroU32::new(5).unwrap(),
            multiplier_bps: 12_500,
            min_fee_rate_delta_sat_vb: NonZeroU64::new(1).unwrap(),
            max_fee_rate_sat_vb: NonZeroU64::new(1_000).unwrap(),
        }
    }

    fn record() -> TxNodeRecord {
        let tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        };
        let mut attempt = TxAttempt::active(
            &tx,
            FeeRate::from_sat_per_vb(10).unwrap(),
            Amount::from_sat(1_000),
            0,
        );
        attempt.first_published_l1_height = Some(100);
        TxNodeRecord::new(TxNodeKind::SingleEnvelopeCommit { payload_idx: 0 }, attempt)
    }

    #[test]
    fn no_bump_before_min_age_blocks() {
        let record = record();
        let active = record.active_attempt().unwrap();

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                active,
                101,
                FeeRate::from_sat_per_vb(20).unwrap(),
                100
            ),
            FeeBumpDecision::Wait
        );
    }

    #[test]
    fn max_attempts_returns_terminal_error() {
        let mut record = record();
        let active = record.active_attempt().unwrap().clone();
        record.attempts.resize(5, active);

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                record.active_attempt().unwrap(),
                102,
                FeeRate::from_sat_per_vb(20).unwrap(),
                100
            ),
            FeeBumpDecision::Terminal(TerminalError::MaxAttemptsReached)
        );
    }

    #[test]
    fn target_fee_chooses_maximum_constraint() {
        let record = record();
        let active = record.active_attempt().unwrap();

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                active,
                102,
                FeeRate::from_sat_per_vb(5).unwrap(),
                100
            ),
            FeeBumpDecision::Replace(FeeBumpRequest {
                target_fee_rate: FeeRate::from_sat_per_vb(13).unwrap(),
                attempt_no: 1,
            })
        );
    }

    #[test]
    fn max_fee_returns_terminal_error() {
        let mut config = config();
        config.max_fee_rate_sat_vb = NonZeroU64::new(12).unwrap();
        let record = record();
        let active = record.active_attempt().unwrap();

        assert_eq!(
            evaluate_fee_bump(
                &config,
                &record,
                active,
                102,
                FeeRate::from_sat_per_vb(5).unwrap(),
                100
            ),
            FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate)
        );
    }
}
