//! Core identifier types and buffer types.

#[macro_use]
mod macros;

mod acct;
mod buf;
mod cred_rule;
mod epoch;
mod exec;
pub mod hash;
mod l1;
mod ol;

pub use acct::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SYSTEM_RESERVED_ACCTS, SubjectId,
};
pub use buf::{Buf20, Buf32, Buf64};
pub use cred_rule::CredRule;
pub use epoch::EpochCommitment;
pub use exec::{
    EVMExtraPayload, EvmEeBlockCommitment, ExecBlockCommitment, create_evm_extra_payload,
};
pub use l1::{BitcoinBlockHeight, L1BlockCommitment, L1BlockId, L1Height};
pub use ol::{L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId, OLTxId};
// Re-export for macro use
#[doc(hidden)]
pub use strata_codec;
