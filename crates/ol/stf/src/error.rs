use strata_acct_types::{AccountId, AcctError};
use strata_ol_chain_types_new::{Epoch, Slot};
use strata_primitives::Buf32;
use strata_snark_acct_types::MessageEntry;
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

    #[error("State root mismatch: expected {expected}, got {got}")]
    StateRootMismatch { expected: Buf32, got: Buf32 },

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

    #[error("Account {0} does not exist")]
    NonExistentAccount(AccountId),

    #[error("Account error: {0}")]
    AccountError(#[from] AcctError),

    #[error("Unsupported transaction type")]
    UnsupportedTransaction,

    #[error(
        "Invalid update sequence for account {account_id}: expected seqno {expected}, got {got}"
    )]
    InvalidUpdateSequence {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error(
        "Invalid message index for account {account_id}: expected new index {expected}, got index {got}"
    )]
    InvalidMsgIndex {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error("Insufficient balance")]
    InsufficientBalance,

    #[error("Message does not exist for account: {account_id}: {message:?}")]
    InvalidMessageProof {
        account_id: AccountId,
        message: MessageEntry,
    },

    #[error("Invalid update proof for account {account_id}")]
    InvalidUpdateProof { account_id: AccountId },

    #[error("{0}")]
    Other(String),

    #[error("Message index overflow for account {account_id}")]
    MsgIndexOverflow { account_id: AccountId },

    #[error("Bitcoin amount overflow")]
    BitcoinAmountOverflow,

    #[error("Epoch overflow: current epoch {cur_epoch}")]
    EpochOverflow { cur_epoch: u64 },
}

pub type StfResult<T> = Result<T, StfError>;
