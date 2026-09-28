//! Log measurements and standalone transaction budget checks for OL.
//!
//! [`LogUsage`] measures log count, payload bytes, and SSZ list size for admission
//! and block assembly. [`check_tx_log_budget`] predicts a transaction's logs and
//! checks whether they fit by themselves, independently of remaining epoch capacity.

mod log_usage;

pub use log_usage::LogUsage;

#[cfg(test)]
mod tests;

use strata_asm_checkpoint_types::MAX_OL_LOGS_PER_CHECKPOINT;
use strata_bridge_params::BridgeParams;
use strata_codec::CodecError;
use strata_identifiers::BRIDGE_GATEWAY_ACCT_ID;
use strata_ol_chain_types_v1::{MAX_LOGS_PER_BLOCK, OLLogType};
use strata_ol_stf::OLSpecId;
use strata_ol_stf::sequencer::parse_bridge_withdrawal;
use strata_ol_tx_types_v1::{OLTransactionV1, TransactionPayloadV1};
use thiserror::Error;

/// Exclusive cap on checkpoint log payload bytes (16 KiB per SPS-ol-chain-structures).
///
/// Reserves room for the state diff and other checkpoint components by bounding
/// aggregate payload bytes. This cap is separate from the per-log size bound and
/// [`MAX_OL_LOGS_PER_CHECKPOINT`]; those schema bounds alone do not guarantee that
/// logs fit this byte budget. Total payload bytes must remain strictly below it.
pub const MAX_TOTAL_LOG_PAYLOAD_BYTES: usize = 16 * 1024;

/// Reports failures to encode predicted logs or fit a standalone transaction log budget.
#[derive(Debug, Error)]
pub enum TxLogBudgetError {
    /// The predicted log count exceeds the reported inclusive maximum.
    #[error("update emits {actual} logs, exceeding limit {limit}")]
    LogCount { actual: usize, limit: usize },

    /// The predicted payload bytes exceed the reported inclusive maximum.
    #[error("update emits {actual} log payload bytes, exceeding limit {limit}")]
    LogPayloadBytes { actual: usize, limit: usize },

    /// Encoding a predicted log payload failed.
    #[error("cannot encode transaction log: {0}")]
    Encoding(#[from] CodecError),
}

fn check_limits(usage: &LogUsage) -> Result<(), TxLogBudgetError> {
    let count_limit = (MAX_LOGS_PER_BLOCK as usize).min(MAX_OL_LOGS_PER_CHECKPOINT as usize - 1);
    if usage.count() > count_limit {
        return Err(TxLogBudgetError::LogCount {
            actual: usage.count(),
            limit: count_limit,
        });
    }
    let payload_limit = MAX_TOTAL_LOG_PAYLOAD_BYTES - 1;
    if usage.payload_bytes() > payload_limit {
        return Err(TxLogBudgetError::LogPayloadBytes {
            actual: usage.payload_bytes(),
            limit: payload_limit,
        });
    }
    Ok(())
}

/// Returns a transaction's predicted log usage if it fits the standalone budget.
///
/// Counts the account-update log and withdrawal logs accepted by the selected STF's
/// bridge rules. Generic account messages return zero usage. This check does not
/// execute the transaction, verify its proof, or establish DA/envelope fit.
///
/// Log count must fit [`MAX_LOGS_PER_BLOCK`] and remain below
/// [`MAX_OL_LOGS_PER_CHECKPOINT`]. Encoded payload bytes must remain below
/// [`MAX_TOTAL_LOG_PAYLOAD_BYTES`], independently of the current epoch's usage.
///
/// # Panics
///
/// Panics if the transaction's SSZ extra-data bound exceeds the account-update log bound.
pub fn check_tx_log_budget(
    spec: OLSpecId,
    tx: &OLTransactionV1,
    bridge_params: &BridgeParams,
) -> Result<LogUsage, TxLogBudgetError> {
    let mut usage = LogUsage::default();
    let TransactionPayloadV1::SnarkAccountUpdate(payload) = tx.payload() else {
        return Ok(usage);
    };
    let update_log = payload
        .operation()
        .update()
        .get_log_data()
        .expect("SSZ update extra data fits the account-update log bound");
    // Use the canonical envelope codec, including its type prefix. Discard
    // each encoded log immediately rather than collecting a transaction's logs.
    usage.add_payload(&update_log.encode_log()?);

    for message in tx.data().effects().messages_iter() {
        if message.dest() != BRIDGE_GATEWAY_ACCT_ID {
            // Non-bridge messages emit no additional OL logs; the account-update
            // log is already counted above.
            continue;
        }
        if let Ok(log) = parse_bridge_withdrawal(
            spec,
            message.payload().value().to_sat(),
            message.payload().data(),
            bridge_params,
        ) {
            usage.add_payload(&log.encode_log()?);
        }
    }
    check_limits(&usage)?;
    Ok(usage)
}
