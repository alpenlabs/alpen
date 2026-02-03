//! Configuration parameters for btcio operations.
//!
//! This module provides [`BtcioParams`] which contains the subset of rollup parameters
//! needed by btcio components.

use strata_l1_txfmt::MagicBytes;

// This struct contains only the parameters that btcio actually needs,
// allowing btcio to be used without depending on the full rollup params.
#[derive(Debug, Clone, Copy)]
pub struct BtcioParams {
    /// L1 reorg safe depth (number of confirmations needed for finality).
    pub l1_reorg_safe_depth: u32,

    /// Magic bytes for OP_RETURN tags in L1 transactions.
    pub magic_bytes: MagicBytes,

    /// Genesis L1 block height (the first L1 block the rollup cares about).
    pub genesis_l1_height: u64,
}

impl BtcioParams {
    /// Creates a new [`BtcioParams`] with the given values.
    pub fn new(l1_reorg_safe_depth: u32, magic_bytes: MagicBytes, genesis_l1_height: u64) -> Self {
        Self {
            l1_reorg_safe_depth,
            magic_bytes,
            genesis_l1_height,
        }
    }

    /// Returns the L1 reorg safe depth.
    pub fn l1_reorg_safe_depth(&self) -> u32 {
        self.l1_reorg_safe_depth
    }

    /// Returns the magic bytes.
    pub fn magic_bytes(&self) -> MagicBytes {
        self.magic_bytes
    }

    /// Returns the genesis L1 block height.
    pub fn genesis_l1_height(&self) -> u64 {
        self.genesis_l1_height
    }
}
