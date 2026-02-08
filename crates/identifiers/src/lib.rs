//! Core identifier types and buffer types.

#[macro_use]
mod macros;

mod acct;
mod buf;
mod cred_rule;
mod deposit;
mod epoch;
mod exec;
pub mod hash;
mod l1;
mod mmr;
mod ol;

use rkyv as _; // rkyv is used in the SSZ schemas

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use acct::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SUBJ_ID_LEN, SYSTEM_RESERVED_ACCTS,
    SubjectId, SubjectIdBytes,
};
pub use buf::{Buf20, Buf32, Buf64};
pub use cred_rule::CredRule;
pub use deposit::{DepositDescriptor, DepositDescriptorError};
pub use exec::{
    EVMExtraPayload, EvmEeBlockCommitment, ExecBlockCommitment, create_evm_extra_payload,
};
pub use hash::Hash;
pub use l1::{BitcoinBlockHeight, L1BlockId, L1Height, WtxidsRoot};
pub use mmr::{MmrId, RawMmrId};
pub use ol::{Epoch, L2BlockCommitment, L2BlockId, OLBlockId, OLTxId, Slot};

// Re-export for macro use
#[doc(hidden)]
#[rustfmt::skip]
pub use strata_codec;

/// SSZ-generated types for serialization and merkleization.
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
pub mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

// Re-export generated commitment types
#[cfg(feature = "bitcoin")]
pub use l1::L1BlockCommitment;
#[cfg(not(feature = "bitcoin"))]
pub use ssz_generated::ssz::commitments::L1BlockCommitment;
pub use ssz_generated::ssz::commitments::{
    EpochCommitment, EpochCommitmentRef, L1BlockCommitmentRef, OLBlockCommitment,
    OLBlockCommitmentRef,
};
