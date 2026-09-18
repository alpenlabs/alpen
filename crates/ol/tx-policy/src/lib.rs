//! Sequencer admission policy for transaction log budgets.
//!
//! Applies the STF's log limits to standalone updates before mempool admission.
//! This does not establish DA fit.

#[cfg(test)]
mod tests;

use strata_acct_types::BRIDGE_GATEWAY_ACCT_ID;
use strata_bridge_params::BridgeParams;
use strata_codec::CodecError;
use strata_ol_chain_types_v1::OLLogType;
use strata_ol_stf_v1::{EpochLogBudgetError, check_epoch_log_budget, parse_bridge_withdrawal};
use strata_ol_tx_types_v1::{OLTransactionV1, TransactionPayloadV1};
use thiserror::Error;

pub use strata_ol_stf_v1::MAX_TOTAL_LOG_PAYLOAD_BYTES;

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

impl From<EpochLogBudgetError> for TxLogBudgetError {
    fn from(error: EpochLogBudgetError) -> Self {
        match error {
            EpochLogBudgetError::LogCount { actual, limit } => Self::LogCount { actual, limit },
            EpochLogBudgetError::LogPayloadBytes { actual, limit } => {
                Self::LogPayloadBytes { actual, limit }
            }
        }
    }
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
        check_epoch_log_budget(self.count, self.payload_bytes)?;
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
/// The transaction effect limits guarantee that its logs fit in one block.
/// Epoch log count and payload bytes must pass [`check_epoch_log_budget`].
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
