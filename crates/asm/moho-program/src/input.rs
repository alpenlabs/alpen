use bitcoin::{Block, hashes::Hash};
use moho_types::StateReference;
use strata_asm_common::AuxData;

/// Private input to process the next state.
///
/// This includes all the L1
#[derive(Clone, Debug)]
pub struct AsmStepInput {
    /// The full Bitcoin L1 block
    pub block: L1Block,
    /// Auxiliary data required to run the ASM STF
    pub aux_data: AuxData,
}

impl AsmStepInput {
    pub fn new(block: L1Block, aux_data: AuxData) -> Self {
        AsmStepInput { block, aux_data }
    }

    /// Computes the state reference.
    ///
    /// In concrete terms, this just computes the blkid/blockhash.
    pub fn compute_ref(&self) -> StateReference {
        let raw_ref = self.block.0.block_hash().to_raw_hash().to_byte_array();
        StateReference::new(raw_ref)
    }

    /// Computes the previous state reference from the input.
    ///
    /// In concrete terms, this just extracts the parent blkid from the block's
    /// header.
    pub fn compute_prev_ref(&self) -> StateReference {
        let parent_ref = self
            .block
            .0
            .header
            .prev_blockhash
            .to_raw_hash()
            .to_byte_array();
        StateReference::new(parent_ref)
    }

    /// Checks that the block's merkle roots are consistent.
    pub fn validate_block(&self) -> bool {
        self.block.0.check_merkle_root() && self.block.0.check_witness_commitment()
    }
}

/// A wrapper around Bitcoin's `Block`.
#[derive(Debug, Clone, PartialEq)]
pub struct L1Block(pub Block);
