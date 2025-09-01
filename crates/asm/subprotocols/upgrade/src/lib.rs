//! Strata Upgrade Subprotocol
//!
//! This module implements the upgrade subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

mod authority;
mod config;
mod constants;
mod error;
mod handler;
mod state;
mod subprotocol;
mod updates;

pub use subprotocol::UpgradeSubprotocol;
