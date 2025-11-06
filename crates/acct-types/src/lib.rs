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
pub use mmr::{CompactMmr64, Hash, MerkleProof, Mmr64, RawMerkleProof, StrataHasher};
pub use ssz_generated::ssz::{
    self as ssz,
    messages::{MsgPayload, ReceivedMessage, SentMessage, SentMessageRef},
    state::{AccountIntrinsicState, AcctStateSummary, EncodedAccountInnerState},
};
pub use state::AccountTypeState;
pub use strata_btc_types::BitcoinAmount;
pub use strata_identifiers::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SubjectId,
};
pub use util::compute_codec_sha256;
pub use varint_vec::{VARINT_MAX, VarVec};

/// Enum representation of system accounts. Provides an `id` method that returns account id.
#[derive(Clone, Debug)]
pub enum SystemAccount {
    Zero,
    Bridge,
}

impl SystemAccount {
    pub fn id(&self) -> AccountId {
        match self {
            SystemAccount::Zero => AccountId::new([0; 32]),
            SystemAccount::Bridge => AccountId::new([1; 32]), // TODO: figure out id
        }
    }
}
