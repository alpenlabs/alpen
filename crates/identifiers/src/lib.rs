//! Core identifier types and buffer types.

#[macro_use]
mod macros;

mod acct;
mod buf;
mod epoch;
mod exec;
mod l1;
mod ol;

#[cfg(feature = "jsonschema")]
mod jsonschema;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub mod hash {
    pub use crate::exec::Hash;
}

pub use acct::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SUBJ_ID_LEN, SYSTEM_RESERVED_ACCTS,
    SubjectId, SubjectIdBytes,
};
pub use buf::{Buf20, Buf32, Buf64, RBuf32};
pub use epoch::EpochCommitment;
#[cfg(feature = "ssz")]
pub use epoch::EpochCommitmentRef;
#[cfg(feature = "borsh")]
pub use exec::create_evm_extra_payload;
pub use exec::{EVMExtraPayload, EvmEeBlockCommitment, ExecBlockCommitment, Hash};
#[cfg(feature = "ssz")]
pub use l1::L1BlockCommitmentRef;
pub use l1::{L1BlockCommitment, L1BlockId, L1Height, WtxidsRoot};
#[cfg(feature = "ssz")]
pub use ol::OLBlockCommitmentRef;
pub use ol::{Epoch, L2BlockCommitment, L2BlockId, OLBlockCommitment, OLBlockId, OLTxId, Slot};
