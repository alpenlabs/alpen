use strata_acct_types::{AccountId, AcctError, BitcoinAmount};
use strata_codec::CodecError;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::{Epoch, Slot};
use strata_ol_da::DaError;
use thiserror::Error;

/// Execution result error.
pub type ExecResult<T> = Result<T, ExecError>;

/// Error from executing/validating the block.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("header epoch does not match state epoch (header {0}, state {1})")]
    EpochMismatch(Epoch, Epoch),

    /// Signature is invalid, for some purpose.
    #[error("signature for {0} is invalid")]
    SignatureInvalid(&'static str),

    #[error("amount overflow")]
    AmountOverflow,

    /// Normal balance check fail.
    #[error("tried to underflow a balance")]
    BalanceUnderflow,

    #[error("condition in tx attachment failed")]
    TxConditionCheckFailed,

    #[error("structural requirement check failed ({0})")]
    TxStructureCheckFailed(&'static str),

    #[error("transaction has expired (max slot {0}, cur slot {1})")]
    TransactionExpired(Slot, Slot),

    #[error("transaction is not mature (min slot {0}, cur slot {1})")]
    TransactionNotMature(Slot, Slot),

    /// For like if we'd be skipping blocks in validation somehow.
    #[error("chain integrity invalid")]
    ChainIntegrity,

    #[error("tried to interact with nonexistent account ({0:?})")]
    UnknownAccount(AccountId),

    /// This is used if the target of a snark account update tx is not a snark
    /// account.
    #[error("tx target invalid for tx type")]
    IncorrectTxTargetType,

    /// Used when the block's body doesn't match its header.
    #[error("internal block structure mismatches")]
    BlockStructureMismatch,

    /// The parent blkid field doesn't match the header we're using to verify
    /// the block.
    #[error("parent blkid mismatch")]
    BlockParentMismatch,

    #[error("verifying genesis header with nonnull parent field")]
    GenesisParentNonnull,

    #[error("genesis-looking block has non-zero slot or epoch field")]
    GenesisCoordsNonzero,

    #[error("tried to skip epoch (parent {0}, current {1})")]
    SkipEpochs(Epoch, Epoch),

    #[error("tried to skip too many slots (parent {0}, current {1})")]
    SkipTooManySlots(Slot, Slot),

    #[error("incorrect epoch sequencing (parent {0}, parent terminal {2}, self {1})")]
    IncorrectEpoch(Epoch, Epoch, bool),

    #[error("incorrect slot (expected {expected}, got {got})")]
    IncorrectSlot { expected: u64, got: u64 },

    #[error("body inconsistent with header terminal flag")]
    InconsistentBodyTerminality,

    #[error("genesis block was not a terminal")]
    GenesisNonterminal,

    #[error("insufficient account balance (acct {0}, need {1})")]
    InsufficientAccountBalance(AccountId, BitcoinAmount),

    #[error("invalid sequence number for account {0} (expected {1}, actual {2})")]
    InvalidSequenceNumber(AccountId, u64, u64),

    #[error("max sequence number reached for account {account_id}")]
    MaxSeqNumberReached { account_id: AccountId },

    #[error("block logs exceeded limit (count {count}, max {max})")]
    LogsOverflow { count: usize, max: usize },

    /// Wrapper to provide additional context about tx processing.
    #[error("tx {0} at idx {1} processing failed: {2}")]
    TxExec(OLTxId, usize, Box<Self>),

    /// Various account errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),

    /// Codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),

    /// DA application error.
    #[error("da: {0}")]
    Da(#[from] DaError),
}

impl ExecError {
    /// Wraps the exec error with context about a transaction in a block.
    pub fn with_tx(self, txid: OLTxId, idx: usize) -> Self {
        Self::TxExec(txid, idx, Box::new(self))
    }

    /// Returns a ref to the base-level error, unwrapping context self-wrappers.
    pub fn base(&self) -> &Self {
        match self {
            Self::TxExec(_, _, inner) => inner.base(),
            _ => self,
        }
    }

    /// Unwraps self-wrappers to expose the base-level error.
    pub fn into_base(self) -> Self {
        match self {
            Self::TxExec(_, _, inner) => *inner,
            _ => self,
        }
    }

    pub fn kind(&self) -> ErrorKind {
        // By default, we can assume all errors indicate the block is invalid,
        // we don't have any execution ones yet.
        ErrorKind::Correctness
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// This is an execution error that should abort validation inconclusively.
    Execution,

    /// This is some correctness error that indicates the block is invalid.
    Correctness,
}
