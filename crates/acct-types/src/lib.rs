//! Account system common type definitions.
#![expect(unused)] // in-development

mod amount;
mod constants;
mod errors;
mod id;
mod macros;
mod messages;
mod mmr;
mod state;

pub use amount::BitcoinAmount;
pub use constants::SYSTEM_RESERVED_ACCTS;
pub use errors::{AcctError, AcctResult};
pub use id::{AcctId, AcctSerial, AcctTypeId, RawAcctTypeId, SubjectId};
pub use messages::{MsgPayload, ReceivedMessage, SentMessage};
pub use mmr::{CompactMmr64, Hash, MerkleProof, Mmr64, RawMerkleProof, StrataHasher};
pub use state::{AcctState, AcctStateSummary, AcctTypeState, IntrinsicAcctState};
