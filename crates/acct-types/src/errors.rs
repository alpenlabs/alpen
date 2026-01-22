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
        "invalid update seqno for snark account {account_id:?} update (expected {expected}, got {got})"
    )]
    InvalidUpdateSequence {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error(
        "invalid next msg index for snark account {account_id:?} update (expected {expected}, got {got})"
    )]
    InvalidMsgIndex {
        account_id: AccountId,
        expected: u64,
        got: u64,
    },

    #[error("insufficient balance for operation (requested {requested}, available {available})")]
    InsufficientBalance {
        requested: BitcoinAmount,
        available: BitcoinAmount,
    },

    #[error("message proof invalid for account {account_id:?} at message index {msg_idx}")]
    InvalidMessageProof { account_id: AccountId, msg_idx: u64 },

    #[error("invalid ledger reference by account {account_id:?} at ref index {ref_idx}")]
    InvalidLedgerReference { account_id: AccountId, ref_idx: u64 },

    #[error("invalid update proof for account {account_id:?}")]
    InvalidUpdateProof { account_id: AccountId },

    #[error("invalid message proofs count for account {account_id:?}")]
    InvalidMsgProofsCount { account_id: AccountId },

    #[error("invalid ledger ref proofs count for account {account_id:?}")]
    InvalidLedgerRefProofsCount { account_id: AccountId },

    #[error(
        "processed message is not the same as proven message for account {account_id:?} at index {msg_index}"
    )]
    InvalidAccumuulatorProofMessageRef {
        account_id: AccountId,
        msg_index: usize,
    },

    #[error("message index overflow for account {account_id:?}")]
    MsgIndexOverflow { account_id: AccountId },

    #[error("bitcoin amount overflow")]
    BitcoinAmountOverflow,

    #[error("operation not supported in this context")]
    Unsupported,
}
