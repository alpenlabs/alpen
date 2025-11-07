use strata_identifiers::{AccountId, AccountTypeId, RawAccountTypeId};
use thiserror::Error;

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

    #[error("invalid account id {0}")]
    InvalidAcctTypeId(RawAccountTypeId),

    // Snark account operational errors
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

    #[error("Insufficient balance in account")]
    InsufficientBalance,

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
}
