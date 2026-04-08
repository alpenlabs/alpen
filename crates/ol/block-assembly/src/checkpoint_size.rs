//! Checkpoint payload size policy used by the sequencer's block assembler.
//!
//! Inlined from alpen's old `strata-checkpoint-types-ssz::validation` module.
//! The asm version of `strata-checkpoint-types-ssz` does not ship a `validation`
//! submodule because the soft/hard size policy is a sequencer concern, not part
//! of the on-chain protocol type schema.
//!
//! The block assembler uses [`checkpoint_size_verdict`] to incrementally check
//! whether the next transaction would push the in-progress checkpoint payload
//! past its hard limit (drop the tx) or past the 90% soft limit (commit the tx
//! and seal the epoch).

use strata_checkpoint_types_ssz::{
    MAX_OL_LOGS_PER_CHECKPOINT, MAX_TOTAL_LOG_PAYLOAD_BYTES, OL_DA_DIFF_MAX_SIZE,
};
use strata_ol_chain_types_new::OLLog;

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

/// Accumulated log metrics for checkpoint size estimation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LogMetrics {
    pub count: usize,
    pub total_payload: usize,
    pub ssz_size: usize,
}

impl LogMetrics {
    pub(crate) fn from_logs(logs: &[OLLog]) -> Self {
        let mut m = Self::default();
        m.add_logs(logs);
        m
    }

    pub(crate) fn add_logs(&mut self, logs: &[OLLog]) {
        for log in logs {
            let payload_len = log.payload.len();
            self.count += 1;
            self.total_payload += payload_len;
            self.ssz_size += 12 + payload_len;
        }
    }
}

/// Decision after checking estimated checkpoint sizes against limits.
///
/// Variants are ordered so `max()` yields the most restrictive verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CheckpointSizeVerdict {
    WithinLimits,
    SoftLimitReached,
    HardLimitExceeded,
}

/// Checks a single dimension against its hard limit and 90% soft threshold.
fn dimension_verdict(value: usize, hard_limit: usize) -> CheckpointSizeVerdict {
    if value >= hard_limit {
        CheckpointSizeVerdict::HardLimitExceeded
    } else if value >= hard_limit * SOFT_LIMIT_RATIO_NUM / SOFT_LIMIT_RATIO_DEN {
        CheckpointSizeVerdict::SoftLimitReached
    } else {
        CheckpointSizeVerdict::WithinLimits
    }
}

/// Computes the overall checkpoint size verdict by checking every dimension.
///
/// `state_diff_size` is the estimated DA diff size.
pub(crate) fn checkpoint_size_verdict(
    state_diff_size: usize,
    log_metrics: &LogMetrics,
) -> CheckpointSizeVerdict {
    let envelope_size = CHECKPOINT_FIXED_OVERHEAD + state_diff_size + log_metrics.ssz_size;

    [
        dimension_verdict(state_diff_size, OL_DA_DIFF_MAX_SIZE as usize),
        dimension_verdict(log_metrics.count, MAX_OL_LOGS_PER_CHECKPOINT as usize),
        dimension_verdict(log_metrics.total_payload, MAX_TOTAL_LOG_PAYLOAD_BYTES),
        dimension_verdict(envelope_size, MAX_CHECKPOINT_PAYLOAD_SIZE),
    ]
    .into_iter()
    .max()
    .unwrap_or(CheckpointSizeVerdict::WithinLimits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_within_limits() {
        let metrics = LogMetrics::default();
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::WithinLimits,
        );
    }

    #[test]
    fn verdict_da_diff_hard_limit() {
        let metrics = LogMetrics::default();
        assert_eq!(
            checkpoint_size_verdict(OL_DA_DIFF_MAX_SIZE as usize, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_da_diff_soft_limit() {
        let metrics = LogMetrics::default();
        let soft = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        assert_eq!(
            checkpoint_size_verdict(soft, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_log_count_hard_limit() {
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_total_payload_hard_limit() {
        let metrics = LogMetrics {
            total_payload: MAX_TOTAL_LOG_PAYLOAD_BYTES + 1,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_envelope_hard_limit() {
        // Construct values that individually are fine but combined exceed envelope.
        let da = OL_DA_DIFF_MAX_SIZE as usize - 1;
        let metrics = LogMetrics {
            ssz_size: MAX_CHECKPOINT_PAYLOAD_SIZE - CHECKPOINT_FIXED_OVERHEAD - da + 1,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(da, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_worst_wins() {
        // DA diff within limits, but log count at hard limit.
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_log_count_soft_limit() {
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_total_payload_soft_limit() {
        let metrics = LogMetrics {
            total_payload: MAX_TOTAL_LOG_PAYLOAD_BYTES * 9 / 10,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_da_diff_soft_with_log_count_within() {
        // DA diff at 90% threshold, log count below threshold.
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize / 2,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(da, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_one_hard_one_soft_yields_hard() {
        // DA diff at soft, log count at hard — worst wins.
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(da, &metrics),
            CheckpointSizeVerdict::HardLimitExceeded,
        );
    }

    #[test]
    fn verdict_envelope_soft_limit() {
        // Components individually fine, but combined SSZ size hits 90% of envelope.
        let envelope_soft = MAX_CHECKPOINT_PAYLOAD_SIZE * 9 / 10;
        let ssz_size = envelope_soft - CHECKPOINT_FIXED_OVERHEAD;
        let metrics = LogMetrics {
            ssz_size,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn log_metrics_empty_logs() {
        let metrics = LogMetrics::from_logs(&[]);
        assert_eq!(metrics, LogMetrics::default());
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
