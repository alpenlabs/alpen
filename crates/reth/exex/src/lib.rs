//! Reth exexes for Alpen EVM: per-block state-diff persistence (DA) and
//! per-block accessed-state capture (consumed by the account proof's
//! range-witness extractor).
//!
//! The per-block *proof-witness* capture used by the chunk proof lives in
//! `alpen-reth-node` (`build_block_witness_from_executed_state`), produced
//! inline during payload build rather than via an exex.

mod accessed_state_exex;
pub mod alloy2reth;
mod state_diff_exex;

pub use accessed_state_exex::AccessedStateGenerator;
pub use state_diff_exex::StateDiffGenerator;
