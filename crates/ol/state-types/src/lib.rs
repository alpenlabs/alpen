//! Ledger data types.
//!
//! This crate is NOT about the basic data structures themselves.  This crate
//! focuses on how we access the ledger data structures in state transition
//! execution contexts.
//!
//! We present a trait that represents the various types of structures we
//! interact with in the ledger's state, and expose accessor functions on it.
//! The different impls of these traits are tailored for different contexts.  In
//! some contexts we care about tracing DA generation, in others we may be doing
//! blocking fetches from disk we want to trace for later proof generation.
//!
//! We use the `I` prefix convention which is normally uncommon in Rust to refer
//! to these abstract data structures.  This is because the "ordinary" struct
//! versions of these data structure we use on the wire are the "real" versions
//! we want to think of them as being, but these traits are standins for those.
//! Making up new names for these items would crate too much confusion.
//!
//! As for structure, this design is based around a "toplevel" state that is not
//! ever actually directly accessed.  Below it, there are two parts:
//!
//! * A "global" state that is treated using the DA framework directly.
//! * An "epochal" state that is only updated in the sealing phase and isn't included in DA.
//! * An "accounts" table, which are selectively loaded.
//!
//! These parts are committed to in the toplevel state, which is updated later
//! when we finish a state transition.
//!
//! The protocol state root commits to [`OLRootState`], which never changes
//! layout: two raw spec versions and the hash tree root of the chainstate. The
//! chainstate layout is versioned separately (`OLStateV1` today), and the
//! layout a state uses is chosen from its current spec.

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
mod coin;
mod errors;
mod pending_asm_log;
mod proofs;
mod root_state;
mod spec_id;
mod state_accessor;

pub use account::*;
pub use coin::{Coin, CoinError};
pub use errors::*;
pub use pending_asm_log::PendingAsmLog;
pub use proofs::*;
pub use spec_id::{OLSpecId, UnknownOLSpecId};
pub use ssz_generated::ssz::root::OLRootState;
pub use state_accessor::*;
// transitional crap
pub use strata_asm_manifest_types::AsmManifest;
pub use strata_identifiers::{EpochCommitment, L1BlockId, L1Height};
