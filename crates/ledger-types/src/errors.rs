use strata_acct_types::{AccountId, AccountSerial, AccountTypeId, AcctError, BitcoinAmount};
use strata_codec::CodecError;
use strata_identifiers::{Epoch, OLTxId, Slot};
use strata_snark_acct_types::Seqno;
use thiserror::Error;

/// State result type returned by state accessor functions.
pub type StateResult<T> = Result<T, StateError>;

/// Errors returned by state accessor functions.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("tried to update non-existent account {0}")]
    MissingAccount(AccountId),

    #[error("sanity check on account existence failed for ID {0}")]
    AccountSanityCheckFail(AccountId),

    #[error("tried to create account with existing ID {0}")]
    AccountExists(AccountId),

    /// Returned when we've only partially loaded the state and can't know if
    /// this account exists or not.
    #[error("tried to access unavailable account {0}")]
    UnavailableAccount(AccountId),

    /// General error indicating insufficient state was provided for this
    /// operation.
    #[error("insufficient state provided")]
    InsufficientState,

    #[error("mismatched account ID (got {got}, exp {expected}")]
    MismatchedAcctType {
        got: AccountTypeId,
        expected: AccountTypeId,
    },

    #[error("insufficient account balance to take (need {need}, have {have}")]
    InsufficientBalance {
        need: BitcoinAmount,
        have: BitcoinAmount,
    },

    #[error("out-of-order seqno change (cur {cur:?}, new {new:?})")]
    OooSeqnoChange { cur: Seqno, new: Seqno },

    #[error("inconsistent acct serial (id {id}, acct {in_acct}, table {in_table})")]
    AcctSerialInconsistent {
        id: AccountId,
        in_acct: AccountSerial,
        in_table: AccountSerial,
    },

    #[error("inconsistent next serial ordering (cur {cur}, new {new})")]
    NextSerialSequence {
        cur: AccountSerial,
        new: AccountSerial,
    },

    #[error("tried to reuse serial {serial} (existing {existing}, new {new})")]
    AccountExistsWithSerial {
        serial: AccountSerial,
        existing: AccountId,
        new: AccountId,
    },
}

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

    #[error("insufficient account balance (acct {id}, need {need})")]
    InsufficientAccountBalance { id: AccountId, need: BitcoinAmount },

    #[error("invalid sequence number for account {id} (expected {exp}, actual {actual})")]
    InvalidSequenceNumber {
        id: AccountId,
        exp: u64,
        actual: u64,
    },

    #[error("max sequence number reached for account {account_id}")]
    MaxSeqNumberReached { account_id: AccountId },

    #[error("block logs exceeded limit (count {count}, max {max})")]
    LogsOverflow { count: usize, max: usize },

    /// Wrapper to provide additional context about tx processing.
    #[error("tx {0} at idx {1} processing failed: {2}")]
    TxExec(OLTxId, usize, Box<Self>),

    /// Errors from chainstate accessor.
    #[error("state access: {0}")]
    State(#[from] StateError),

    /// Various account errors.
    #[error("acct: {0}")]
    Acct(#[from] AcctError),

    /// Codec error.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
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
