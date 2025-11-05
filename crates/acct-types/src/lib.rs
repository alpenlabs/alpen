//! Account system common type definitions.

// Re-export for macro use
#[doc(hidden)]
pub use strata_codec;

mod constants;
mod errors;
mod macros;
mod messages;
mod mmr;
mod state;
mod util;
mod varint_vec;

pub use constants::SYSTEM_RESERVED_ACCTS;
pub use errors::{AcctError, AcctResult};
pub use messages::{MsgPayload, ReceivedMessage, SentMessage};
pub use mmr::{CompactMmr64, Hash, MerkleProof, Mmr64, RawMerkleProof, StrataHasher};
pub use state::{AccountState, AccountTypeState, AcctStateSummary, IntrinsicAccountState};
pub use strata_btc_types::BitcoinAmount;
pub use strata_identifiers::{
    AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SubjectId,
};
pub use util::compute_codec_sha256;
pub use varint_vec::{VARINT_MAX, VarVec};
