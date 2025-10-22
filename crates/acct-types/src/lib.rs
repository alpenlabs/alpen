//! Account system common type definitions.

// Re-export for macro use
#[doc(hidden)]
pub use strata_codec;

mod amount;
mod constants;
mod errors;
mod id;
mod macros;
mod messages;
mod mmr;
mod state;
mod util;
mod varint_vec;

pub use amount::BitcoinAmount;
pub use constants::SYSTEM_RESERVED_ACCTS;
pub use errors::{AcctError, AcctResult};
pub use id::{AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SubjectId};
pub use messages::{MsgPayload, MsgPayloadData, ReceivedMessage, SentMessage};
pub use mmr::{CompactMmr64, Hash, MerkleProof, Mmr64, RawMerkleProof, StrataHasher};
pub use state::{
    AccountEncodedState, AccountState, AccountTypeState, AcctStateSummary, IntrinsicAccountState,
};
pub use util::compute_codec_sha256;
pub use varint_vec::{VARINT_MAX, VarVec};

// Re-export SSZ constants
pub const MAX_MSG_PAYLOAD_DATA_BYTES: usize = 1 << 20; // 1 MiB
pub const MAX_ACCOUNT_ENCODED_STATE_BYTES: usize = 1 << 16; // 64 KiB
