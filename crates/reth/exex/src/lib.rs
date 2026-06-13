//! Reth exexes for Alpen EVM: per-block state-diff persistence (DA) and
//! per-block accessed-state capture (consumed by the account proof's
//! range-witness extractor).
//!
//! The per-block *proof-witness* capture used by the chunk proof now lives in
//! `alpen-reth-witness` (`capture_block_witness` / `CacheDBProvider`), produced
//! inline in block production rather than via an exex.

mod accessed_state_exex;
pub mod alloy2reth;
mod state_diff_exex;

pub use accessed_state_exex::AccessedStateGenerator;
pub use state_diff_exex::StateDiffGenerator;
