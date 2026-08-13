//! Fee bumping policy calculation.

use bitcoin::{Amount, FeeRate};
use strata_config::btcio::FeeBumpingConfig;
use strata_db_types::fee_bump::{TerminalError, TxAttempt, TxAttemptStatus, TxNodeRecord};
use strata_primitives::L1Height;

/// A concrete fee-bump request for one active transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FeeBumpRequest {
    /// Replacement fee rate.
    pub target_fee_rate: FeeRate,
    /// Attempt number to assign to the replacement.
    pub attempt_no: u32,
}

/// Policy decision for one transaction node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeeBumpDecision {
    /// The transaction is not eligible for replacement yet.
    Wait,
    /// The transaction should be replaced.
    Replace(FeeBumpRequest),
    /// The current fee estimate is above the effective ceiling.
    ///
    /// Deliberately not [`Self::Terminal`]. The estimate is the one input to the target that
    /// recovers on its own, so a fee-market spike must leave the chain bumpable once the market
    /// settles. Reported rather than silently folded into [`Self::Wait`] so an operator whose
    /// ceiling is holding back every bump can see it.
    BlockedByCeiling {
        /// The estimate that exceeded the ceiling.
        estimate: FeeRate,
        /// The lower of the configured ceiling and a reveal's fundable ceiling.
        ceiling: FeeRate,
    },
    /// The replacement chain cannot advance further.
    Terminal(TerminalError),
}

/// Runtime inputs used to evaluate one replacement candidate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FeeBumpEvaluation {
    pub current_l1_tip: L1Height,
    pub estimate_fee_rate: FeeRate,
    pub incremental_relay_fee_rate: FeeRate,
    pub replacement_vsize: usize,
    /// Maximum absolute replacement fee a reveal can fund while preserving its dust output.
    pub reveal_fee_budget: Option<Amount>,
}

/// Evaluates whether an active published transaction should be replaced.
pub(crate) fn evaluate_fee_bump(
    config: &FeeBumpingConfig,
    record: &TxNodeRecord,
    active_attempt: &TxAttempt,
    evaluation: FeeBumpEvaluation,
) -> FeeBumpDecision {
    let FeeBumpEvaluation {
        current_l1_tip,
        estimate_fee_rate,
        incremental_relay_fee_rate,
        replacement_vsize,
        reveal_fee_budget,
    } = evaluation;
    let Some(first_published_height) = active_attempt.first_published_l1_height else {
        return FeeBumpDecision::Wait;
    };

    let age = current_l1_tip.saturating_sub(first_published_height);
    if age < config.min_age_blocks.get() {
        return FeeBumpDecision::Wait;
    }

    // Discarded attempts were never broadcast, so they must not consume the budget.
    let broadcast_attempts = record
        .attempts
        .iter()
        .filter(|attempt| attempt.status != TxAttemptStatus::Discarded)
        .count();
    if broadcast_attempts >= config.max_attempts.get() as usize {
        return FeeBumpDecision::Terminal(TerminalError::MaxAttemptsReached);
    }

    let FeeFloors {
        additive,
        multiplicative,
        bip125_min,
        max_fee_rate,
    } = match replacement_fee_floors(
        config,
        active_attempt,
        incremental_relay_fee_rate,
        replacement_vsize,
    ) {
        Ok(floors) => floors,
        Err(error) => return FeeBumpDecision::Terminal(error),
    };

    // Split the ceiling check by whether the constraint that breached it can ever ease.
    //
    // The floors below are all derived from the active attempt, and within a replacement chain the
    // active fee rate only ever rises, so a floor above the ceiling stays above it for good. Those
    // are genuinely terminal. The market estimate is re-read on every pass and falls back on its
    // own, so terminating on it would let one fee spike disable bumping for this transaction
    // permanently, which is precisely the situation the bumper exists for.
    let policy_floor = additive.max(multiplicative).max(bip125_min);
    let (effective_ceiling, reveal_headroom_binds) = match reveal_fee_budget {
        Some(budget) => {
            let replacement_vsize_sat = u64::try_from(replacement_vsize).unwrap_or(u64::MAX);
            let fundable_rate_sat_vb = budget
                .to_sat()
                .checked_div(replacement_vsize_sat)
                .unwrap_or(0);
            let fundable_rate =
                FeeRate::from_sat_per_vb(fundable_rate_sat_vb).unwrap_or(max_fee_rate);
            (
                max_fee_rate.min(fundable_rate),
                fundable_rate < max_fee_rate,
            )
        }
        None => (max_fee_rate, false),
    };

    if policy_floor > effective_ceiling {
        // Distinguish "our own escalation policy outgrew the ceiling" from "BIP-125 alone already
        // demands more than the ceiling allows", because only the latter is a relay-rule dead end.
        let error = if reveal_headroom_binds {
            TerminalError::RevealFeeHeadroomExhausted
        } else if bip125_min > max_fee_rate {
            TerminalError::Bip125FeeRuleUnsatisfiable
        } else {
            TerminalError::AboveMaxFeeRate
        };
        return FeeBumpDecision::Terminal(error);
    }

    let target = estimate_fee_rate.max(policy_floor);
    if target > effective_ceiling {
        return FeeBumpDecision::BlockedByCeiling {
            estimate: estimate_fee_rate,
            ceiling: effective_ceiling,
        };
    }

    FeeBumpDecision::Replace(FeeBumpRequest {
        target_fee_rate: target,
        attempt_no: record.next_attempt_no(),
    })
}

/// The fee rates a replacement has to clear, plus the ceiling they are judged against.
struct FeeFloors {
    /// Active rate plus the configured minimum delta.
    additive: FeeRate,
    /// Active rate scaled by `multiplier_bps`.
    multiplicative: FeeRate,
    /// The rate implied by BIP-125 rule 4's absolute-fee floor.
    bip125_min: FeeRate,
    /// The configured `max_fee_rate_sat_vb`, carried along because deriving it is one of the same
    /// conversions and every caller needs it right after.
    max_fee_rate: FeeRate,
}

/// Derives the floors a replacement must clear from the active attempt and the node's relay fee.
///
/// Every failure here is a conversion that cannot fit [`FeeRate`], which means the rate involved is
/// already absurd, so they are all terminal.
fn replacement_fee_floors(
    config: &FeeBumpingConfig,
    active_attempt: &TxAttempt,
    incremental_relay_fee_rate: FeeRate,
    replacement_vsize: usize,
) -> Result<FeeFloors, TerminalError> {
    let active_fee_rate = FeeRate::from_sat_per_vb(active_attempt.fee_rate_sat_vb)
        .ok_or(TerminalError::AboveMaxFeeRate)?;
    let min_fee_rate_delta = FeeRate::from_sat_per_vb(config.min_fee_rate_delta_sat_vb.get())
        .ok_or(TerminalError::AboveMaxFeeRate)?;
    let max_fee_rate = FeeRate::from_sat_per_vb(config.max_fee_rate_sat_vb.get())
        .ok_or(TerminalError::AboveMaxFeeRate)?;

    let active_fee_rate_sat_vb = active_fee_rate.to_sat_per_vb_ceil();
    let additive = FeeRate::from_sat_per_vb(
        active_fee_rate_sat_vb.saturating_add(min_fee_rate_delta.to_sat_per_vb_ceil()),
    )
    .ok_or(TerminalError::AboveMaxFeeRate)?;

    let multiplicative_fee_rate_sat_vb = active_fee_rate_sat_vb
        .saturating_mul(config.multiplier_bps as u64)
        .div_ceil(10_000);
    let multiplicative = FeeRate::from_sat_per_vb(multiplicative_fee_rate_sat_vb)
        .ok_or(TerminalError::AboveMaxFeeRate)?;

    // BIP-125 rule 4 is priced at the node's own `incrementalrelayfee`, which is runtime
    // configurable. Take the larger of that and the operator's configured delta so a node running
    // a raised relay fee does not reject every replacement we build.
    let bip125_relay_fee_rate = incremental_relay_fee_rate.max(min_fee_rate_delta);
    let bip125_min = bip125_minimum_fee_rate(
        Amount::from_sat(active_attempt.fee_sats),
        bip125_relay_fee_rate,
        replacement_vsize,
    )
    .ok_or(TerminalError::Bip125FeeRuleUnsatisfiable)?;

    Ok(FeeFloors {
        additive,
        multiplicative,
        bip125_min,
        max_fee_rate,
    })
}

/// Converts the BIP-125 absolute-fee floor into the replacement's fee rate.
pub(crate) fn bip125_minimum_fee_rate(
    active_fee: Amount,
    incremental_relay_fee_rate: FeeRate,
    replacement_vsize: usize,
) -> Option<FeeRate> {
    if replacement_vsize == 0 {
        return Some(FeeRate::ZERO);
    }
    let relay_fee = incremental_relay_fee_rate.fee_vb(replacement_vsize as u64)?;
    let required_fee = active_fee.checked_add(relay_fee)?;
    FeeRate::from_sat_per_vb(required_fee.to_sat().div_ceil(replacement_vsize as u64))
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroU64};

    use bitcoin::{absolute::LockTime, transaction::Version, Amount, Transaction};
    use strata_config::btcio::FeeBumpingConfig;
    use strata_db_types::fee_bump::{TxAttempt, TxNodeId, TxNodeKind, TxNodeRecord};

    use super::*;
    use crate::tx_attempt::attempt_parts;

    fn config() -> FeeBumpingConfig {
        FeeBumpingConfig {
            check_interval_ms: NonZeroU64::new(30_000).unwrap(),
            min_age_blocks: NonZeroU32::new(2).unwrap(),
            max_attempts: NonZeroU32::new(5).unwrap(),
            multiplier_bps: 12_500,
            min_fee_rate_delta_sat_vb: NonZeroU64::new(1).unwrap(),
            max_fee_rate_sat_vb: NonZeroU64::new(1_000).unwrap(),
            max_reveal_fee_headroom_sats: NonZeroU64::new(10_000_000).unwrap(),
        }
    }

    fn evaluation(
        current_l1_tip: L1Height,
        estimate_fee_rate: FeeRate,
        incremental_relay_fee_rate: FeeRate,
        replacement_vsize: usize,
    ) -> FeeBumpEvaluation {
        FeeBumpEvaluation {
            current_l1_tip,
            estimate_fee_rate,
            incremental_relay_fee_rate,
            replacement_vsize,
            reveal_fee_budget: None,
        }
    }

    fn reveal_evaluation(
        estimate_fee_rate: FeeRate,
        replacement_vsize: usize,
        reveal_fee_budget: Amount,
    ) -> FeeBumpEvaluation {
        FeeBumpEvaluation {
            reveal_fee_budget: Some(reveal_fee_budget),
            ..evaluation(
                102,
                estimate_fee_rate,
                FeeRate::from_sat_per_vb(1).unwrap(),
                replacement_vsize,
            )
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
            attempt_parts(
                &tx,
                FeeRate::from_sat_per_vb(10).unwrap(),
                Amount::from_sat(1_000),
            ),
            0,
        );
        attempt.first_published_l1_height = Some(100);
        TxNodeRecord::new(TxNodeKind::SingleEnvelopeCommit { payload_idx: 0 }, attempt)
    }

    fn record_of_kind(kind: TxNodeKind) -> TxNodeRecord {
        let mut record = record();
        record.node_id = TxNodeId::from_kind(&kind);
        record.kind = kind;
        record
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
                evaluation(
                    101,
                    FeeRate::from_sat_per_vb(20).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
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
                evaluation(
                    102,
                    FeeRate::from_sat_per_vb(20).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
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
                evaluation(
                    102,
                    FeeRate::from_sat_per_vb(5).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
            ),
            FeeBumpDecision::Replace(FeeBumpRequest {
                target_fee_rate: FeeRate::from_sat_per_vb(13).unwrap(),
                attempt_no: 1,
            })
        );
    }

    /// A fee spike must not permanently disable bumping. With the active attempt at 10 sat/vB and
    /// a ceiling of 20, the additive (11), multiplicative (13) and BIP-125 floors all fit under the
    /// ceiling; only the market estimate does not. That has to stay retryable.
    #[test]
    fn estimate_above_ceiling_is_retryable_not_terminal() {
        let mut config = config();
        config.max_fee_rate_sat_vb = NonZeroU64::new(20).unwrap();
        let record = record();
        let active = record.active_attempt().unwrap();

        assert_eq!(
            evaluate_fee_bump(
                &config,
                &record,
                active,
                evaluation(
                    102,
                    FeeRate::from_sat_per_vb(500).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
            ),
            FeeBumpDecision::BlockedByCeiling {
                estimate: FeeRate::from_sat_per_vb(500).unwrap(),
                ceiling: FeeRate::from_sat_per_vb(20).unwrap(),
            }
        );
    }

    /// The same chain becomes bumpable again once the estimate falls back under the ceiling,
    /// which is the property the terminal error destroyed.
    #[test]
    fn chain_bumps_again_after_the_estimate_falls_back() {
        let mut config = config();
        config.max_fee_rate_sat_vb = NonZeroU64::new(20).unwrap();
        let record = record();
        let active = record.active_attempt().unwrap();

        let spiked = evaluate_fee_bump(
            &config,
            &record,
            active,
            evaluation(
                102,
                FeeRate::from_sat_per_vb(500).unwrap(),
                FeeRate::from_sat_per_vb(1).unwrap(),
                100,
            ),
        );
        assert!(matches!(spiked, FeeBumpDecision::BlockedByCeiling { .. }));

        assert_eq!(
            evaluate_fee_bump(
                &config,
                &record,
                active,
                evaluation(
                    102,
                    FeeRate::from_sat_per_vb(5).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
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
                evaluation(
                    102,
                    FeeRate::from_sat_per_vb(5).unwrap(),
                    FeeRate::from_sat_per_vb(1).unwrap(),
                    100,
                )
            ),
            FeeBumpDecision::Terminal(TerminalError::AboveMaxFeeRate)
        );
    }

    /// A discarded attempt was built but never broadcast, so it cost the chain nothing and must
    /// not count against `max_attempts`. Without this the budget silently shrinks every time an
    /// external signature is abandoned.
    #[test]
    fn discarded_attempts_do_not_consume_the_attempt_budget() {
        let mut record = record();
        let active = record.active_attempt().unwrap().clone();

        // Fill the budget, then mark all but the active attempt discarded.
        record.attempts.resize(5, active);
        for attempt in record.attempts.iter_mut().skip(1) {
            attempt.status = TxAttemptStatus::Discarded;
        }

        assert!(
            matches!(
                evaluate_fee_bump(
                    &config(),
                    &record,
                    record.active_attempt().unwrap(),
                    evaluation(
                        102,
                        FeeRate::from_sat_per_vb(20).unwrap(),
                        FeeRate::from_sat_per_vb(1).unwrap(),
                        100,
                    )
                ),
                FeeBumpDecision::Replace(_)
            ),
            "four discarded attempts must leave the budget open"
        );
    }

    #[test]
    fn stale_reveal_with_ample_budget_is_replaceable() {
        let record = record_of_kind(TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx: 0,
            reveal_idx: 0,
        });

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                record.active_attempt().unwrap(),
                reveal_evaluation(
                    FeeRate::from_sat_per_vb(5).unwrap(),
                    100,
                    Amount::from_sat(5_000),
                ),
            ),
            FeeBumpDecision::Replace(FeeBumpRequest {
                target_fee_rate: FeeRate::from_sat_per_vb(13).unwrap(),
                attempt_no: 1,
            })
        );
    }

    #[test]
    fn legacy_shaped_reveal_exhausts_its_zero_headroom_budget() {
        let record = record_of_kind(TxNodeKind::SingleEnvelopeReveal { payload_idx: 0 });

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                record.active_attempt().unwrap(),
                reveal_evaluation(
                    FeeRate::from_sat_per_vb(5).unwrap(),
                    100,
                    Amount::from_sat(1_000),
                ),
            ),
            FeeBumpDecision::Terminal(TerminalError::RevealFeeHeadroomExhausted)
        );
    }

    #[test]
    fn reveal_recovers_after_estimate_falls_below_fundable_ceiling() {
        let record = record_of_kind(TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx: 0,
            reveal_idx: 0,
        });
        let budget = Amount::from_sat(2_000);

        assert_eq!(
            evaluate_fee_bump(
                &config(),
                &record,
                record.active_attempt().unwrap(),
                reveal_evaluation(FeeRate::from_sat_per_vb(500).unwrap(), 100, budget),
            ),
            FeeBumpDecision::BlockedByCeiling {
                estimate: FeeRate::from_sat_per_vb(500).unwrap(),
                ceiling: FeeRate::from_sat_per_vb(20).unwrap(),
            }
        );

        assert!(matches!(
            evaluate_fee_bump(
                &config(),
                &record,
                record.active_attempt().unwrap(),
                reveal_evaluation(FeeRate::from_sat_per_vb(5).unwrap(), 100, budget),
            ),
            FeeBumpDecision::Replace(_)
        ));
    }
}
