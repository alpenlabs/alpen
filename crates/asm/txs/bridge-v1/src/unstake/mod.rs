//! Unstake Transaction Parser
//!
//! This module provides functionality for parsing Bitcoin unstaking intent transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Note on Terminology
//!
//! In the ASM context, "unstake transaction" refers to the **unstaking intent** transaction.
//! While there is a separate unstaking transaction that actually transfers funds back to the
//! operator, ASM only parses and validates the unstaking intent transaction. Once the unstaking
//! intent transaction is observed on-chain, the operator is immediately removed from bridge
//! duties, regardless of whether the final unstaking transaction is seen.
//!
//! ## Unstake Transaction Structure
//!
//! An unstaking intent transaction is posted by an operator when it wants to exit from bridge
//! duties and have its staked funds returned.
//!
//! ### Inputs
//! - 1. **Stake connector**: Locked to the N-of-N multisig and hashlock.
//!
//! ### Outputs
//!
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with
//!     - Magic number (4 bytes): Protocol instance identifier
//!     - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!     - Transaction type (1 byte): Unstake transaction type
//!     - Auxiliary data (4 bytes):
//!         - Operator index (4 bytes, encoded using [`strata_codec::Codec`] which uses big-endian)

mod aux;
mod info;
mod parse;
mod script;

pub use aux::UnstakeTxHeaderAux;
pub use info::UnstakeInfo;
pub use parse::{STAKE_INPUT_INDEX, parse_unstake_tx};
pub use script::{stake_connector_script, validate_and_extract_script_params};
