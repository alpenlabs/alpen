//! Strata Upgrade Subprotocol
//!
//! This module implements the upgrade subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations.

pub mod crypto;
pub mod error;
pub mod multisig;
pub mod roles;
pub mod state;
pub mod subprotocol;
pub mod txs;
pub mod upgrades;
