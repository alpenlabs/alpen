//! Unstake Transaction Parser
//!
//! This module provides functionality for parsing Bitcoin unstake transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Unstake Transaction Structure
//!
//! An unstake transaction is posted by an operator if it wants to exit from bridge duties and
//! have its staked funds back.
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
//!
//! Additional output sends the stake to the operator, but ASM skips validating them because
//! correctness is assumed to be enforced during presigning as they spend from the same N/N
//! multisig.

mod aux;
mod info;
mod parse;

pub use aux::UnstakeTxHeaderAux;
pub use info::UnstakeInfo;
pub use parse::{STAKE_INPUT_INDEX, parse_unstake_tx};
