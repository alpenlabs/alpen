//! Strata Administration Subprotocol
//!
//! This module implements the administration subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

mod authority;
mod config;
mod error;
mod handler;
mod queued_update;
pub mod state;
mod subprotocol;

pub use config::AdministrationSubprotoParams;
pub use subprotocol::AdministrationSubprotocol;
