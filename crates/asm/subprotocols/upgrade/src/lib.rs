//! Strata Upgrade Subprotocol
//!
//! This module implements the upgrade subprotocol for Strata, providing
//! on-chain governance and time-delayed enactment of multisig-backed
//! configuration changes, verifying key updates, operator set changes,
//! sequencer updates, and cancellations. Each upgrade proposal undergoes
//! a three-stage lifecycle: initialization (proposal + quorum voting),
//! enactment delay (time-locked period), and final enactment or cancellation.

pub mod actions;
pub mod crypto;
pub mod error;
pub mod roles;
pub mod state;
pub mod subprotocol;
pub mod vote;
