//! Account system common type definitions.

// Re-export for macro use
#[doc(hidden)]
pub use strata_codec;
#[doc(hidden)]
pub use tree_hash;

mod constants;
mod errors;
mod macros;
mod messages;
mod mmr;
mod state;
mod util;
mod varint_vec;

// Include generated SSZ types from build.rs output
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use constants::SYSTEM_RESERVED_ACCTS;
pub use errors::{AcctError, AcctResult};
pub use mmr::{
    CompactMmr64, CompactMmr64Ref, MerkleProof, MerkleProofRef, Mmr64, Mmr64Ref, RawMerkleProof,
    RawMerkleProofRef, StrataHasher,
};
pub use ssz_generated::ssz::{
    self as ssz,
    messages::{MsgPayload, MsgPayloadRef, ReceivedMessage, SentMessage, SentMessageRef},
    state::{AccountIntrinsicState, AcctStateSummary, EncodedAccountInnerState},
};
pub use state::AccountTypeState;
pub use strata_btc_types::BitcoinAmount;
pub use strata_identifiers::{
    AccountId, AccountSerial, AccountTypeId, Hash, RawAccountTypeId, SubjectId,
};
pub use util::compute_codec_sha256;
pub use varint_vec::{VARINT_MAX, VarVec};
