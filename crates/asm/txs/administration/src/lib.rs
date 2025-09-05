//! Strata Administration Transaction Definitions and Parsing Logic
//!
//! This module provides transaction types, parsing utilities, and constants for the Strata
//! Administration Subprotocol. It handles multisig-backed governance transactions that propose
//! and manage time-delayed configuration changes to the protocol.
//!
//! ## Transaction Types
//!
//! The administration subprotocol supports the following transaction types:
//!
//! - **Cancel Transaction** (`CANCEL_TX_TYPE = 0`): Cancels a previously queued update
//! - **Multisig Config Update** (`MULTISIG_CONFIG_UPDATE_TX_TYPE = 10`): Updates multisignature
//!   configuration
//! - **Operator Set Update** (`OPERATOR_UPDATE_TX_TYPE = 11`): Updates the set of authorized
//!   operators
//! - **Sequencer Update** (`SEQUENCER_UPDATE_TX_TYPE = 12`): Updates sequencer configuration
//! - **Verifying Key Update** (`VK_UPDATE_TX_TYPE = 13`): Updates the protocol's verifying key
//!
//! ## Core Structures
//!
//! - [`actions::MultisigAction`]: High-level multisig operations that can be proposed (Cancel or
//!   Update)
//! - [`actions::CancelAction`]: Specific action to cancel a pending update by ID
//! - [`actions::UpdateAction`]: Various update types (multisig, operator, sequencer, verifying key)
//! - [`strata_crypto::multisig::vote::AggregatedVote`]: Cryptographic signature aggregation for
//!   multisig voting

pub mod actions;
pub mod constants;
pub mod error;
pub mod parser;
