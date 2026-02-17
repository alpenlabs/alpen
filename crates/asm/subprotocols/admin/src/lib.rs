//! Strata Administration Subprotocol
//!
//! This module implements the administration subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

mod authority;
mod error;
mod handler;
mod queued_update;
mod state;
mod subprotocol;

pub use state::AdministrationSubprotoState;
pub use subprotocol::AdministrationSubprotocol;
