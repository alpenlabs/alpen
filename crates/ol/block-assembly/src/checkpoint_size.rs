//! Checkpoint payload size policy used by the sequencer's block assembler.
//!
//! Inlined from alpen's old `strata-asm-proto-checkpoint-types::validation` module.
//! The asm version of `strata-asm-proto-checkpoint-types` does not ship a `validation`
//! submodule because the soft/hard size policy is a sequencer concern, not part
//! of the on-chain protocol type schema.
//!
//! The block assembler uses [`checkpoint_size_verdict`] to incrementally check
//! whether the next transaction would violate a checkpoint hard limit (defer the tx)
//! or the 90% soft threshold (commit the tx and seal the epoch).

use strata_asm_checkpoint_types::{MAX_OL_LOGS_PER_CHECKPOINT, OL_DA_DIFF_MAX_SIZE};
use strata_ol_log_budget::{LogUsage, MAX_TOTAL_LOG_PAYLOAD_BYTES};

/// L1 envelope limit for the full `CheckpointPayload` (single envelope, not chunked).
pub(crate) const MAX_CHECKPOINT_PAYLOAD_SIZE: usize = 395_000;

/// Fixed overhead in the `CheckpointPayload` SSZ encoding.
///
/// ```text
/// CheckpointPayload SSZ layout:
///   PAYLOAD_FIXED (60)  = CheckpointTip(52) + sidecar_offset(4) + proof_offset(4)
///   SIDECAR_FIXED (112) = state_diff_offset(4) + logs_offset(4) + TerminalHeaderComplement(104)
///   + ol_state_diff bytes (variable)
///   + ol_logs bytes       (per-log: 4 offset + 4 account_serial + 4 payload_offset + payload)
///   + proof bytes         (worst case MAX_PROOF_LEN = 4 KiB)
///   + CodecSsz varint     (≤5 bytes)
/// ```
pub(crate) const CHECKPOINT_FIXED_OVERHEAD: usize = {
    const PAYLOAD_FIXED: usize = 60;
    const SIDECAR_FIXED: usize = 112;
    const PROOF_BUDGET: usize = 4096;
    const CODEC_OVERHEAD: usize = 5;
    PAYLOAD_FIXED + SIDECAR_FIXED + PROOF_BUDGET + CODEC_OVERHEAD
};

const SOFT_LIMIT_RATIO_NUM: usize = 9;
const SOFT_LIMIT_RATIO_DEN: usize = 10;

/// Checkpoint size and count limits, each with a hard and soft threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckpointLimit {
    DaDiff,
    LogCount,
    LogPayloadBytes,
    Envelope,
}

/// Decision after checking a checkpoint size or count against its limit.
///
/// Variants are ordered so `max()` yields the most restrictive verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CheckpointSizeVerdict {
    WithinLimits,
    SoftLimitReached,
    HardLimitExceeded,
}

/// Checks checkpoint usage against the selected limit's hard and soft thresholds.
///
/// Log-count and payload-byte limits are inclusive; DA-diff and envelope limits
/// remain exclusive. `state_diff_size` is the estimated DA diff size.
pub(crate) fn checkpoint_size_verdict(
    limit: CheckpointLimit,
    state_diff_size: usize,
    log_usage: &LogUsage,
) -> CheckpointSizeVerdict {
    let (value, hard_limit) = match limit {
        CheckpointLimit::DaDiff => (state_diff_size, OL_DA_DIFF_MAX_SIZE as usize),
        CheckpointLimit::LogCount => (log_usage.count(), MAX_OL_LOGS_PER_CHECKPOINT as usize),
        CheckpointLimit::LogPayloadBytes => {
            (log_usage.payload_bytes(), MAX_TOTAL_LOG_PAYLOAD_BYTES)
        }
        CheckpointLimit::Envelope => (
            CHECKPOINT_FIXED_OVERHEAD + state_diff_size + log_usage.ssz_size(),
            MAX_CHECKPOINT_PAYLOAD_SIZE,
        ),
    };
    let hard_limit_exceeded = match limit {
        CheckpointLimit::LogCount | CheckpointLimit::LogPayloadBytes => value > hard_limit,
        CheckpointLimit::DaDiff | CheckpointLimit::Envelope => value >= hard_limit,
    };
    if hard_limit_exceeded {
        CheckpointSizeVerdict::HardLimitExceeded
    } else if value >= hard_limit * SOFT_LIMIT_RATIO_NUM / SOFT_LIMIT_RATIO_DEN {
        CheckpointSizeVerdict::SoftLimitReached
    } else {
        CheckpointSizeVerdict::WithinLimits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::make_log_usage;

    #[test]
    fn verdict_within_limits() {
        let metrics = LogUsage::default();
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::DaDiff, 0, &metrics),
            CheckpointSizeVerdict::WithinLimits,
        );
    }

    #[test]
    fn verdict_da_diff_hard_limit() {
        let metrics = LogUsage::default();
        assert_eq!(
            checkpoint_size_verdict(
                CheckpointLimit::DaDiff,
                OL_DA_DIFF_MAX_SIZE as usize,
                &metrics
            ),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_da_diff_soft_limit() {
        let metrics = LogUsage::default();
        let soft = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::DaDiff, soft, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_log_count_hard_limit() {
        let metrics = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize + 1, 0);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogCount, 0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_total_payload_hard_limit() {
        let mut metrics = make_log_usage(4, MAX_TOTAL_LOG_PAYLOAD_BYTES / 4);
        metrics.add_payload(&[0]);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogPayloadBytes, 0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_envelope_hard_limit() {
        // Construct values that individually fit but together reach the envelope limit.
        let metrics = make_log_usage(12_000, 0);
        let da = MAX_CHECKPOINT_PAYLOAD_SIZE - CHECKPOINT_FIXED_OVERHEAD - metrics.ssz_size();
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::Envelope, da, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_log_count_hard_with_da_within() {
        // DA diff within limits, but log count exceeds the hard limit.
        let metrics = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize + 1, 0);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogCount, 0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_log_count_soft_limit() {
        let metrics = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10, 0);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogCount, 0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_total_payload_soft_limit() {
        let soft = MAX_TOTAL_LOG_PAYLOAD_BYTES * 9 / 10;
        let mut metrics = make_log_usage(4, soft / 4);
        metrics.add_payload(&vec![0; soft % 4]);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogPayloadBytes, 0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_da_diff_soft_with_log_count_within() {
        // DA diff at 90% threshold, log count below threshold.
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        let metrics = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize / 2, 0);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::DaDiff, da, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_log_count_hard_with_da_soft() {
        // DA diff at soft does not hide the hard log-count limit.
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        let metrics = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize + 1, 0);
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::LogCount, da, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_allows_exact_log_limits() {
        let mut usage = make_log_usage(MAX_OL_LOGS_PER_CHECKPOINT as usize - 4, 0);
        for _ in 0..4 {
            usage.add_payload(&[0; MAX_TOTAL_LOG_PAYLOAD_BYTES / 4]);
        }
        for limit in [CheckpointLimit::LogCount, CheckpointLimit::LogPayloadBytes] {
            assert_eq!(
                checkpoint_size_verdict(limit, 0, &usage),
                CheckpointSizeVerdict::SoftLimitReached,
            );
        }
    }

    #[test]
    fn verdict_envelope_soft_limit() {
        // Components individually fine, but combined SSZ size hits 90% of envelope.
        let envelope_soft = MAX_CHECKPOINT_PAYLOAD_SIZE * 9 / 10;
        let metrics = make_log_usage(10_000, 0);
        let da = envelope_soft - CHECKPOINT_FIXED_OVERHEAD - metrics.ssz_size();
        assert_eq!(
            checkpoint_size_verdict(CheckpointLimit::Envelope, da, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn thresholds_are_consistent() {
        const { assert!(CHECKPOINT_FIXED_OVERHEAD < MAX_CHECKPOINT_PAYLOAD_SIZE) };
    }

    // Pin protocol constants so accidental changes break the build.
    #[test]
    fn pinned_constants() {
        const { assert!(OL_DA_DIFF_MAX_SIZE == 1 << 18) }; // 256 KiB
        const { assert!(MAX_OL_LOGS_PER_CHECKPOINT == 1 << 14) }; // 16,384
        const { assert!(MAX_TOTAL_LOG_PAYLOAD_BYTES == 16 * 1024) }; // 16 KiB
        const { assert!(MAX_CHECKPOINT_PAYLOAD_SIZE == 395_000) };
        const { assert!(CHECKPOINT_FIXED_OVERHEAD == 4273) };
        const { assert!(SOFT_LIMIT_RATIO_NUM == 9) };
        const { assert!(SOFT_LIMIT_RATIO_DEN == 10) };
    }
}
