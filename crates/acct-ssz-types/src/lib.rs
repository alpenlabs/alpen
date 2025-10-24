//! SSZ types for account system, defined per the pythonic schema in `schemas/acct-types.ssz`.
//!
//! Types here match the schema exactly. Constants are used by `strata-acct-types` which
//! contains the same types plus business logic. In the future, types will be auto-generated
//! via ssz-gen, and `strata-acct-types` will re-export them with extension traits.

mod id;
mod messages;
mod state;

// Re-export constants defined in schema
pub const MAX_ACCOUNT_ENCODED_STATE_BYTES: usize = 1 << 16; // 64 KiB (2^16)
pub const MAX_MSG_PAYLOAD_DATA_BYTES: usize = 1 << 20; // 1 MiB (2^20)

// Re-export BitcoinAmount from strata-btc-types
// This matches the schema definition: class BitcoinAmount(uint64)
// Re-export all types
pub use id::{AccountId, AccountSerial, AccountTypeId, RawAccountTypeId, SubjectId};
pub use messages::{MsgPayload, MsgPayloadData, ReceivedMessage, SentMessage};
pub use state::{AccountEncodedState, AccountState, AcctStateSummary, IntrinsicAccountState};
pub use strata_btc_types::BitcoinAmount;
