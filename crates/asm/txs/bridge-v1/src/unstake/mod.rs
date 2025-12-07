//! Unstake Transaction Parser
//!
//! This module provides functionality for parsing Bitcoin unstake transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Unstake Transaction Structure
//!
//! An unstake transaction is posted by an operator if they want to exit from the bridge and have
//! their staked fund back.
//!
//! ### Inputs
//! - 1. **Unstaking Intent connector**: Locked to the N-of-N multisig with a relative timelock
//! - 2. **Stake connector**: Locked to the N-of-N multisig..
//!
//! Only the stake connector is validated. The unstaking intent connector carries a relative
//! timelock that is enforced on-chain, but the bridge subprotocol does not store the timelock that
//! is used, so ASM cannot verify it and skips validation of that input. This is sufficient because
//! the transaction is identified by its SPS-50 type; a different transaction that merely spends a
//! pure N-of-N input would fail the type check.
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
