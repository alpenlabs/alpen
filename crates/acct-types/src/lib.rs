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
mod varint_vec;

pub use amount::BitcoinAmount;
pub use constants::SYSTEM_RESERVED_ACCTS;
pub use errors::{AcctError, AcctResult};
pub use id::{AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SubjectId};
pub use messages::{MsgPayload, ReceivedMessage, SentMessage};
pub use mmr::{CompactMmr64, Hash, MerkleProof, Mmr64, RawMerkleProof, StrataHasher};
pub use state::{AccountState, AccountTypeState, AcctStateSummary, IntrinsicAccountState};
pub use varint_vec::{VARINT_MAX, VarVec};
