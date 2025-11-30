//! Slash Transaction Parser
//!
//! This module provides functionality for parsing Bitcoin slash transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Slash Transaction Structure
//!
//! A slash transaction is posted by a watchtower if the claim was contested but the operator hasn't
//! posted the Contested Payout Transaction within the allotted time frame. The transaction spends
//! the operator stake and distributes it to watchtowers.
//!
//! ### Inputs
//! - 1. **Contest slash connector**: Locked to the N-of-N multisig with a relative timelock
//! - 2. **Stake connector**: Locked to the N-of-N pure multisig, without any timelocks.
//!
//! Only the stake connector is validated. The contest connector carries a relative timelock that
//! is enforced on-chain, but the bridge subprotocol does not store the timelock that is used,
//! so ASM cannot verify it and skips validation of that input. This is sufficient because the
//! transaction is identified by its SPS-50 type; a different transaction that merely spends a
//! pure N-of-N input would fail the type check.
//!
//! ### Outputs
//!
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with
//!     - Magic number (4 bytes): Protocol instance identifier
//!     - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!     - Transaction type (1 byte): Slash transaction type
//!     - Auxiliary data (4 bytes):
//!         - Operator index (4 bytes, encoded using [`strata_codec::Codec`] which uses big-endian)
//!
//! Additional outputs distribute the stake to watchtowers (e.g., change), but ASM skips validating
//! them because correctness is assumed to be enforced during presigning and they spend from the
//! same N/N multisig.

mod aux;
mod info;
mod parse;

pub use aux::SlashTxHeaderAux;
pub use info::SlashInfo;
pub use parse::parse_slash_tx;
