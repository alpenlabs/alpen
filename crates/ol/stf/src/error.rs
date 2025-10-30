use strata_acct_types::AcctError;
use strata_ol_chain_types_new::{Epoch, Slot};
use strata_primitives::Buf32;
use strata_snark_acct_types::MessageEntry;

// TODO: use thiserror
/// Errors related to block validation.
#[derive(Clone, Debug)]
pub enum BlockValidationError {
    SlotMismatch {
        expected: Slot,
        got: Slot,
    },
    BlockIdMismatch {
        expected: Buf32,
        got: Buf32,
    },
    InvalidEpoch(Epoch),
    InvalidTimestamp(u64),
    MismatchedBodyRoot {
        expected: Buf32,
        got: Buf32,
    },
    InvalidSignature,
    StateRootMismatch {
        expected: strata_primitives::Buf32,
        got: strata_primitives::Buf32,
    },
    LogsRootMismatch {
        expected: strata_primitives::Buf32,
        got: strata_primitives::Buf32,
    },
}

/// All errors related to stf.
#[derive(Debug)]
pub enum StfError {
    BlockValidation(BlockValidationError),
    InvalidAsmLog,
    NonExistentAccount(strata_acct_types::AccountId),
    AccountError(AcctError),
    UnsupportedTransaction,
    InvalidUpdateSequence,
    InvalidMsgIndex,
    InsufficientBalance,
    NonExistentMessage(MessageEntry), /* FIXME: this is perhaps too big of a variant
                                       * TODO: might also need acct id/serial */
}

impl From<AcctError> for StfError {
    fn from(value: AcctError) -> Self {
        StfError::AccountError(value)
    }
}

pub type StfResult<T> = Result<T, StfError>;
