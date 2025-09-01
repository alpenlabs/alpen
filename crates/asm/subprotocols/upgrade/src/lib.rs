//! Strata Upgrade Subprotocol
//!
//! This module implements the upgrade subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

pub mod authority;
pub mod constants;
pub mod error;
pub mod handler;
pub mod state;
pub mod subprotocol;
pub mod updates;
