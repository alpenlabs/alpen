//! Transaction effect and block log capacities, by spec.

use strata_acct_types::{MAX_MESSAGES, MAX_TRANSFERS, TxEffects};
use strata_ol_chain_types_v1::MAX_LOGS_PER_BLOCK;
use strata_ol_state_types::{ExecError, OLSpecId, TxEffectsOverflow};
use strata_ol_stf_v1::ExecResult;

use crate::spec::{OLStfSpec, dispatch_spec};
use crate::v0::StfV0;
use crate::v1::StfV1;
use crate::v2::StfV2;

/// The transaction effect and block log capacities a spec's rules allow.
///
/// Every spec from V1 on uses the V1 transaction and block types. Their SSZ
/// bounds allow 65,536 transfers and 65,536 messages per transaction, and the
/// V1 rules allow [`MAX_LOGS_PER_BLOCK`] logs per block. The effect bounds
/// also set the depth of the txid's Merkle tree and the longest lists a node
/// decodes, so they keep their V1 values. A spec that allows less checks its
/// capacities as a validity rule instead. Drivers that build blocks or admit
/// transactions read the capacities here.
///
/// All capacities are inclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OLSpecLimits {
    max_tx_transfers: usize,
    max_tx_messages: usize,
    max_block_logs: usize,
}

impl OLSpecLimits {
    /// The capacities of the 0.3.0 rules, which V0 names.
    pub(crate) const V0: Self = Self {
        max_tx_transfers: 255,
        max_tx_messages: 255,
        max_block_logs: 1 << 12,
    };

    /// The capacities of the V1 rules: the bounds of the V1 types.
    pub(crate) const V1: Self = Self {
        max_tx_transfers: MAX_TRANSFERS as usize,
        max_tx_messages: MAX_MESSAGES as usize,
        max_block_logs: MAX_LOGS_PER_BLOCK as usize,
    };

    /// The capacities of the V2 rules, which restore the V0 values.
    ///
    /// A transaction emits one log per bridge withdrawal message plus one for a
    /// snark account update, so a transaction at these capacities always fits
    /// a block.
    pub(crate) const V2: Self = Self::V0;

    /// Returns the most transfers a transaction may carry.
    pub fn max_tx_transfers(&self) -> usize {
        self.max_tx_transfers
    }

    /// Returns the most messages a transaction may carry.
    pub fn max_tx_messages(&self) -> usize {
        self.max_tx_messages
    }

    /// Returns the most logs a block may emit, its epoch-terminal logs
    /// included.
    pub fn max_block_logs(&self) -> usize {
        self.max_block_logs
    }

    /// Checks a transaction's effects against the transfer and message
    /// capacities.
    ///
    /// # Errors
    ///
    /// Returns [`TxEffectsOverflow`] for the first list above its capacity,
    /// transfers before messages.
    pub fn check_tx_effects(&self, effects: &TxEffects) -> Result<(), TxEffectsOverflow> {
        let count = effects.transfers_iter().count();
        if count > self.max_tx_transfers {
            return Err(TxEffectsOverflow::Transfers {
                count,
                max: self.max_tx_transfers,
            });
        }
        let count = effects.messages_iter().count();
        if count > self.max_tx_messages {
            return Err(TxEffectsOverflow::Messages {
                count,
                max: self.max_tx_messages,
            });
        }
        Ok(())
    }

    /// Checks a block's log count against the block log capacity.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::LogsOverflow`] if `count` exceeds the capacity.
    pub fn check_block_log_count(&self, count: usize) -> ExecResult<()> {
        if count > self.max_block_logs {
            return Err(ExecError::LogsOverflow {
                count,
                max: self.max_block_logs,
            });
        }
        Ok(())
    }
}

/// Returns the transaction effect and block log capacities the rules of
/// `spec` allow.
pub fn spec_limits(spec: OLSpecId) -> OLSpecLimits {
    dispatch_spec!(spec => limits())
}
