//! Strata Administration Subprotocol
//!
//! This module implements the administration subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

use arbitrary as _;

mod authority;
mod error;
mod handler;
mod queued_update;
mod state;
mod subprotocol;

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

pub use ssz_generated::ssz::state::{MultisigAuthority, QueuedUpdate};
pub use state::AdministrationSubprotoState;
pub use subprotocol::AdministrationSubprotocol;
