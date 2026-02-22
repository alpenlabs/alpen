//! Strata Administration Transaction Definitions and Parsing Logic
//!
//! This module provides transaction types, parsing utilities, and constants for the Strata
//! Administration Subprotocol. It handles multisig-backed governance transactions that propose
//! and manage time-delayed configuration changes to the protocol.
//!
//! ## Transaction Types
//!
//! See [`constants::AdminTxType`] for the full list of supported transaction types.
//!
//! ## Core Structures
//!
//! - [`actions::MultisigAction`]: High-level multisig operations that can be proposed (Cancel or
//!   Update)
//! - [`actions::CancelAction`]: Specific action to cancel a pending update by ID
//! - [`actions::UpdateAction`]: Various update types (multisig, operator, sequencer, verifying key)

pub mod actions;
pub mod constants;
pub mod errors;
pub mod parser;
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
