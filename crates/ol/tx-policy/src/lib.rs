//! Sequencer admission policy for transaction log budgets.
//!
//! These checks retain the assembler's exclusive checkpoint thresholds. They
//! constrain admission, not consensus validity, and do not establish DA fit.

#[cfg(test)]
mod tests;

use strata_acct_types::BRIDGE_GATEWAY_ACCT_ID;
use strata_asm_checkpoint_types::MAX_OL_LOGS_PER_CHECKPOINT;
use strata_bridge_params::BridgeParams;
use strata_codec::CodecError;
use strata_ol_chain_types_v1::{MAX_LOGS_PER_BLOCK, OLLogType};
use strata_ol_stf_v1::parse_bridge_withdrawal;
use strata_ol_tx_types_v1::{OLTransactionV1, TransactionPayloadV1};
use thiserror::Error;

/// Maximum total OL log payload size per checkpoint (16 KiB per SPS-ol-chain-structures).
///
/// Set well below the full checkpoint envelope limit to reserve room for the
/// checkpoint's other components (e.g. state diff), by bounding logs in
/// aggregate rather than lowering `MAX_LOG_PAYLOAD_LEN` (per-log) or
/// [`MAX_OL_LOGS_PER_CHECKPOINT`]. It is intentionally separate from,
/// and inconsistent with, those two limits.
///
/// This threshold is exclusive: total log payload size must be strictly below it.
pub const MAX_TOTAL_LOG_PAYLOAD_BYTES: usize = 16 * 1024;

/// Failures when measuring or checking a transaction's own log budget.
#[derive(Debug, Error)]
pub enum TxLogBudgetError {
    /// The emitted log count exceeds the inclusive per-update limit.
    #[error("update emits {actual} logs, exceeding limit {limit}")]
    LogCount { actual: usize, limit: usize },
    /// The encoded log payloads exceed the inclusive per-update byte limit.
    #[error("update emits {actual} log payload bytes, exceeding limit {limit}")]
    LogPayloadBytes { actual: usize, limit: usize },
    /// A typed log could not be encoded.
    #[error("cannot encode transaction log: {0}")]
    Encoding(#[from] CodecError),
}

/// Log usage of a transaction that passes the standalone admission budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxLogUsage {
    count: usize,
    payload_bytes: usize,
}

impl TxLogUsage {
    /// Returns the number of emitted logs, including the account-update log.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Returns encoded payload bytes, excluding OLLog container framing.
    pub fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }

    fn add_log(&mut self, log: &impl OLLogType) -> Result<(), CodecError> {
        self.count += 1;
        // Use the canonical envelope codec, including its type prefix. Discard
        // each encoded log immediately rather than collecting a transaction's logs.
        self.payload_bytes += log.encode_log()?.len();
        Ok(())
    }

    fn check_limits(self) -> Result<Self, TxLogBudgetError> {
        let count_limit =
            (MAX_LOGS_PER_BLOCK as usize).min(MAX_OL_LOGS_PER_CHECKPOINT as usize - 1);
        if self.count > count_limit {
            return Err(TxLogBudgetError::LogCount {
                actual: self.count,
                limit: count_limit,
            });
        }
        let payload_limit = MAX_TOTAL_LOG_PAYLOAD_BYTES - 1;
        if self.payload_bytes > payload_limit {
            return Err(TxLogBudgetError::LogPayloadBytes {
                actual: self.payload_bytes,
                limit: payload_limit,
            });
        }
        Ok(self)
    }
}

/// Checks a snark update's log usage independently of the remaining epoch capacity.
///
/// Counts logs that a successful execution would emit, using the STF's bridge
/// classification and the configured bridge parameters. This does not execute
/// the update, verify its proof, or check its DA/envelope footprint. Valid generic
/// account messages emit no logs and pass this policy unchanged.
///
/// The log count must fit [`MAX_LOGS_PER_BLOCK`] and remain strictly below
/// [`MAX_OL_LOGS_PER_CHECKPOINT`]. Combined encoded log payload bytes must remain
/// strictly below [`MAX_TOTAL_LOG_PAYLOAD_BYTES`].
///
/// # Panics
///
/// Panics if the SSZ extra-data bound stops fitting the account-update log bound.
pub fn check_tx_log_budget(
    tx: &OLTransactionV1,
    bridge_params: &BridgeParams,
) -> Result<TxLogUsage, TxLogBudgetError> {
    let mut usage = TxLogUsage {
        count: 0,
        payload_bytes: 0,
    };
    let TransactionPayloadV1::SnarkAccountUpdate(payload) = tx.payload() else {
        return Ok(usage);
    };
    let update_log = payload
        .operation()
        .update()
        .get_log_data()
        .expect("SSZ update extra data fits the account-update log bound");
    usage.add_log(&update_log)?;

    for message in tx.data().effects().messages_iter() {
        if message.dest() != BRIDGE_GATEWAY_ACCT_ID {
            // Non-bridge messages emit no additional OL logs; the account-update
            // log is already counted above. Inter-EE inbox writes contribute to
            // checkpoint DA.
            continue;
        }
        if let Ok(log) = parse_bridge_withdrawal(
            message.payload().value().to_sat(),
            message.payload().data(),
            bridge_params,
        ) {
            usage.add_log(&log)?;
        }
    }
    usage.check_limits()
}
