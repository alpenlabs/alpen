//! Core identifier types and buffer types.

#[macro_use]
mod macros;

mod acct;
mod buf;
mod epoch;
mod exec;
pub mod hash;
mod l1;
mod ol;

#[cfg(feature = "jsonschema")]
mod jsonschema;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use acct::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SUBJ_ID_LEN, SYSTEM_RESERVED_ACCTS,
    SubjectId, SubjectIdBytes, SubjectIdBytesError,
};
pub use buf::{Buf20, Buf32, Buf64, RBuf32};
pub use exec::{
    EVMExtraPayload, EvmEeBlockCommitment, ExecBlockCommitment, create_evm_extra_payload,
};
pub use hash::Hash;
pub use l1::{L1BlockId, L1Height, WtxidsRoot};
pub use ol::{
    Epoch, L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockCommitmentRef, OLBlockId,
    OLTxId, Slot,
};

// Re-export for macro use
#[doc(hidden)]
#[rustfmt::skip]
pub use strata_codec;

pub use epoch::{EpochCommitment, EpochCommitmentRef};
pub use l1::{L1BlockCommitment, L1BlockCommitmentRef};
