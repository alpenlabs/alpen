use strata_identifiers::{AccountId, AccountSerial};
use thiserror::Error;

use crate::{AccountTypeId, BitcoinAmount};

pub type AcctResult<T> = Result<T, AcctError>;

/// Account related error types.
// leaving this abbreviated because it's used a lot
#[derive(Debug, Error)]
pub enum AcctError {
    /// When we mismatch uses of types.
    ///
    /// (real acct type, asked type)
    #[error("tried to use {0} as {1}")]
    MismatchedType(AccountTypeId, AccountTypeId),

    /// Issue decoding an account's type state.
    #[error("decode {0} account state")]
    DecodeState(AccountTypeId),

    #[error("tried to create account with existing ID ({0:?})")]
    AccountIdExists(AccountId),

    #[error("tried to access account that does not exist ({0:?})")]
    MissingExpectedAccount(AccountId),

    #[error("tried to create account with serial {0} but next serial is {1}")]
    SerialSequence(AccountSerial, AccountSerial),

    #[error("account {0} has serial {1} but tried to insert as serial idx {2}")]
    AccountSerialInconsistent(AccountId, AccountSerial, AccountSerial),

    #[error("tried to create new account with existing ID {0}")]
    CreateExistingAccount(AccountId),

    #[error("tried to non-create update non-existent account with ID {0}")]
    UpdateNonexistentAccount(AccountId),

    #[error(
        "Invalid update sequence for account {account_id:?}: expected seqno {expected}, got {got}"
    )]
    InvalidUpdateSequence {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error(
        "Invalid message index for account {account_id:?}: expected new index {expected}, got index {got}"
    )]
    InvalidMsgIndex {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error("Insufficient balance in account. Requested {requested}, available {available} ")]
    InsufficientBalance {
        requested: BitcoinAmount,
        available: BitcoinAmount,
    },

    #[error("Message proof invalid for account {account_id:?} at message index {msg_idx}")]
    InvalidMessageProof { account_id: AccountId, msg_idx: u64 },

    #[error("Invalid ledger reference by account {account_id:?} at ref index {ref_idx}")]
    InvalidLedgerReference { account_id: AccountId, ref_idx: u64 },

    #[error("Invalid update proof for account {account_id:?}")]
    InvalidUpdateProof { account_id: AccountId },

    #[error("Message index overflow for account {account_id:?}")]
    MsgIndexOverflow { account_id: AccountId },

    #[error("Bitcoin amount overflow")]
    BitcoinAmountOverflow,

    #[error("Account {0:?} does not exist")]
    NonExistentAccount(AccountId),

    #[error("operation not supported in this context")]
    Unsupported,
}
