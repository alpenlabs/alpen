use strata_acct_types::{AccountId, AcctError, strata_codec::CodecError};
use strata_primitives::{Buf32, Epoch, Slot};
use thiserror::Error;

/// Errors related to block validation.
#[derive(Debug, Error)]
pub enum BlockValidationError {
    #[error("Slot mismatch: expected {expected}, got {got}")]
    SlotMismatch { expected: Slot, got: Slot },

    #[error("Block ID mismatch: expected {expected}, got {got}")]
    BlockIdMismatch { expected: Buf32, got: Buf32 },

    #[error("Invalid epoch: {0}")]
    InvalidEpoch(Epoch),

    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(u64),

    #[error("Mismatched body root: expected {expected}, got {got}")]
    MismatchedBodyRoot { expected: Buf32, got: Buf32 },

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Pre-state root mismatch: expected {expected}, got {got}")]
    PreStateRootMismatch { expected: Buf32, got: Buf32 },

    #[error("Post-state root mismatch: expected {expected}, got {got}")]
    PostStateRootMismatch { expected: Buf32, got: Buf32 },

    #[error("Logs root mismatch: expected {expected}, got {got}")]
    LogsRootMismatch { expected: Buf32, got: Buf32 },
}

/// All errors related to stf.
#[derive(Debug, Error)]
pub enum StfError {
    #[error("Block validation failed: {0}")]
    BlockValidation(#[from] BlockValidationError),

    #[error("Invalid ASM log")]
    InvalidAsmLog,

    #[error("Account error: {0}")]
    AccountError(#[from] AcctError),

    #[error("Unsupported transaction type")]
    UnsupportedTransaction,

    #[error("{0}")]
    UnsupportedTransfer(String),

    #[error("Epoch overflow: current epoch {cur_epoch}")]
    EpochOverflow { cur_epoch: Epoch },

    #[error("Unsupported transfer to {0:?}")]
    UnsupportedTransferTo(AccountId),

    #[error("codec error: {0}")]
    CodecError(#[from] CodecError),

    #[error("Preseal root mismatch. Expected {expected}, got {got}")]
    PresealRootMismatch { expected: Buf32, got: Buf32 },
}

pub type StfResult<T> = Result<T, StfError>;
