//! OL state types with required V1 fields enforced during SSZ decoding.
//!
//! Public states store mandatory fields directly in private domain structs.
//! Ordinary [`ssz::Decode`] rejects missing required fields when converting the
//! generated wire representation into a domain value. SSZ operations use the
//! upstream [`strata_identifiers::SszDelegate`] implementation.
//!
//! The delegate conversion moves fields into present wire slots for encoding.
//! State accessors and mutations operate directly on the required domain fields.

// TODO(STR-4122): Before closing this ticket, create a follow-up ticket to adopt upstream
// borrowed SSZ delegation when available, avoiding state clones during encoding, encoded-size
// calculation, and tree hashing.

// Include generated SSZ types from build.rs output
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

mod account;
mod batch_application;
mod constants;
mod epochal;
mod global;
mod intraepoch;
mod ledger;
mod protocol;
mod required_fields;
mod serial_map;
mod snark_account;
mod toplevel;
mod write_batch;

#[cfg(test)]
mod stable_container_tests;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

// Re-export SSZ-generated types that are used publicly
pub use account::OLAccountStateV1;
pub use batch_application::*;
pub use constants::*;
pub use epochal::EpochalStateV1;
pub use global::GlobalStateV1;
pub use intraepoch::IntraepochStateV1;
pub use protocol::ProtocolStateV1;
pub use serial_map::*;
pub use snark_account::OLSnarkAccountStateV1;
pub use ssz_generated::ssz::state::{
    MAX_ACCOUNT_SERIALS, MAX_LEDGER_ACCOUNTS, MAX_PENDING_ASM_LOGS, OLAccountTypeStateV1,
    OLAccountTypeStateV1Ref, PendingAsmLogEntryV1, PendingAsmLogEntryV1Ref, ProofStateV1,
    ProofStateV1Ref, TsnlAccountEntryV1, TsnlAccountEntryV1Ref, TsnlLedgerAccountsTableV1,
    TsnlLedgerAccountsTableV1Ref,
};
pub use toplevel::OLStateV1;
pub use write_batch::*;
