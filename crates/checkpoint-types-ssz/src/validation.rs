use strata_ol_chain_types_new::OLLog;

use crate::{
    CheckpointPayloadError, MAX_OL_LOGS_PER_CHECKPOINT, MAX_TOTAL_LOG_PAYLOAD_BYTES,
    OL_DA_DIFF_MAX_SIZE,
};

/// L1 envelope limit for the full `CheckpointPayload` (single envelope, not chunked).
pub const MAX_CHECKPOINT_PAYLOAD_SIZE: usize = 395_000;

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
pub const CHECKPOINT_FIXED_OVERHEAD: usize = {
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
pub struct LogMetrics {
    pub count: usize,
    pub total_payload: usize,
    pub ssz_size: usize,
}

impl LogMetrics {
    pub fn from_logs(logs: &[OLLog]) -> Self {
        let mut m = Self::default();
        m.add_logs(logs);
        m
    }

    pub fn add_logs(&mut self, logs: &[OLLog]) {
        for log in logs {
            let payload_len = log.payload().len();
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
pub enum CheckpointSizeVerdict {
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
/// `state_diff_size` is the estimated DA diff size (from
/// [`EpochDaAccumulator::estimated_encoded_size`]).
pub fn checkpoint_size_verdict(
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

/// Hard-limit-only validation for [`CheckpointSidecar`](crate::CheckpointSidecar) construction.
pub fn validate_checkpoint_components(
    state_diff_size: usize,
    log_count: usize,
    total_log_payload: usize,
) -> Result<(), CheckpointPayloadError> {
    if state_diff_size as u64 > OL_DA_DIFF_MAX_SIZE {
        return Err(CheckpointPayloadError::StateDiffTooLarge {
            provided: state_diff_size as u64,
            max: OL_DA_DIFF_MAX_SIZE,
        });
    }
    if log_count as u64 > MAX_OL_LOGS_PER_CHECKPOINT {
        return Err(CheckpointPayloadError::OLLogsTooLarge {
            provided: log_count as u64,
            max: MAX_OL_LOGS_PER_CHECKPOINT,
        });
    }
    if total_log_payload > MAX_TOTAL_LOG_PAYLOAD_BYTES {
        return Err(CheckpointPayloadError::OLLogsTotalPayloadTooLarge {
            provided: total_log_payload as u64,
            max: MAX_TOTAL_LOG_PAYLOAD_BYTES as u64,
        });
    }
    Ok(())
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
    fn validate_components_ok() {
        assert!(validate_checkpoint_components(0, 0, 0).is_ok());
    }

    #[test]
    fn validate_components_rejects_large_diff() {
        let result = validate_checkpoint_components(OL_DA_DIFF_MAX_SIZE as usize + 1, 0, 0);
        assert!(matches!(
            result,
            Err(CheckpointPayloadError::StateDiffTooLarge { .. })
        ));
    }

    #[test]
    fn validate_components_rejects_too_many_logs() {
        let result =
            validate_checkpoint_components(0, MAX_OL_LOGS_PER_CHECKPOINT as usize + 1, 0);
        assert!(matches!(
            result,
            Err(CheckpointPayloadError::OLLogsTooLarge { .. })
        ));
    }

    #[test]
    fn validate_components_rejects_large_total_payload() {
        let result = validate_checkpoint_components(0, 0, MAX_TOTAL_LOG_PAYLOAD_BYTES + 1);
        assert!(matches!(
            result,
            Err(CheckpointPayloadError::OLLogsTotalPayloadTooLarge { .. })
        ));
    }

    #[test]
    fn log_metrics_incremental() {
        let log1 = OLLog::new(strata_identifiers::AccountSerial::one(), vec![0u8; 100]);
        let log2 = OLLog::new(strata_identifiers::AccountSerial::one(), vec![0u8; 200]);

        let batch = LogMetrics::from_logs(&[log1.clone(), log2.clone()]);

        let mut incremental = LogMetrics::default();
        incremental.add_logs(&[log1]);
        incremental.add_logs(&[log2]);

        assert_eq!(batch, incremental);
        assert_eq!(batch.count, 2);
        assert_eq!(batch.total_payload, 300);
        assert_eq!(batch.ssz_size, 2 * 12 + 300);
    }

    #[test]
    fn verdict_log_count_soft_limit() {
        let soft = MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10;
        let metrics = LogMetrics {
            count: soft,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_total_payload_soft_limit() {
        let soft = MAX_TOTAL_LOG_PAYLOAD_BYTES * 9 / 10;
        let metrics = LogMetrics {
            total_payload: soft,
            ..Default::default()
        };
        assert_eq!(
            checkpoint_size_verdict(0, &metrics),
            CheckpointSizeVerdict::SoftLimitReached,
        );
    }

    #[test]
    fn verdict_just_below_all_soft_thresholds() {
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10 - 1;
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10 - 1,
            total_payload: MAX_TOTAL_LOG_PAYLOAD_BYTES * 9 / 10 - 1,
            ssz_size: 0,
        };
        // Envelope = CHECKPOINT_FIXED_OVERHEAD + da + 0, well under 395k.
        assert_eq!(
            checkpoint_size_verdict(da, &metrics),
            CheckpointSizeVerdict::WithinLimits,
        );
    }

    #[test]
    fn verdict_multiple_soft_limits_still_soft() {
        // DA diff and log count both at soft threshold — worst is still SoftLimitReached.
        let da = OL_DA_DIFF_MAX_SIZE as usize * 9 / 10;
        let metrics = LogMetrics {
            count: MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10,
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
}
