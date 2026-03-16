use bitcoin::{
    Block,
    consensus::{deserialize, serialize},
    hashes::Hash,
};
use moho_types::StateReference;
use ssz::{Decode, Encode};
use strata_asm_common::AuxData;

use crate::{AsmStepInput, AuxDataBytes, BlockBytes};

/// Private input to process the next state transition.
#[derive(Debug, Clone, PartialEq)]
pub struct L1Block(pub Block);

impl AsmStepInput {
    fn encode_aux_data(aux_data: AuxData) -> AuxDataBytes {
        AuxDataBytes::new(aux_data.as_ssz_bytes())
            .expect("moho-program aux data must fit in SSZ bounds")
    }

    fn decode_aux_data(bytes: &[u8]) -> AuxData {
        AuxData::from_ssz_bytes(bytes).expect("moho-program aux data bytes must remain valid")
    }

    fn encode_block(block: L1Block) -> BlockBytes {
        serialize(&block.0).into()
    }

    fn decode_block(bytes: &[u8]) -> L1Block {
        let block = deserialize(bytes).expect("moho-program block bytes must remain valid");
        L1Block(block)
    }

    /// Creates a new Moho step input.
    pub fn new(block: L1Block, aux_data: AuxData) -> Self {
        Self {
            block: Self::encode_block(block),
            aux_data: Self::encode_aux_data(aux_data),
        }
    }

    /// Returns the full Bitcoin L1 block.
    pub fn block(&self) -> L1Block {
        Self::decode_block(&self.block)
    }

    /// Returns the auxiliary data required for the ASM STF.
    pub fn aux_data(&self) -> AuxData {
        Self::decode_aux_data(&self.aux_data)
    }

    /// Computes the state reference.
    ///
    /// In concrete terms, this just computes the blkid/blockhash.
    pub fn compute_ref(&self) -> StateReference {
        let raw_ref = self.block().0.block_hash().to_raw_hash().to_byte_array();
        StateReference::new(raw_ref)
    }

    /// Computes the previous state reference from the input.
    ///
    /// In concrete terms, this just extracts the parent blkid from the block's
    /// header.
    pub fn compute_prev_ref(&self) -> StateReference {
        let block = self.block();
        let parent_ref = block.0.header.prev_blockhash.to_raw_hash().to_byte_array();
        StateReference::new(parent_ref)
    }

    /// Checks that the block's merkle roots are consistent.
    pub fn validate_block(&self) -> bool {
        let block = self.block();
        block.0.check_merkle_root() && block.0.check_witness_commitment()
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_test_utils_btc::segment::BtcChainSegment;

    use super::*;

    #[test]
    fn test_ssz_roundtrip() {
        let block = BtcChainSegment::load_full_block();
        let input = AsmStepInput::new(L1Block(block), AuxData::new(vec![], vec![]));

        let serialized = input.as_ssz_bytes();
        let decoded = AsmStepInput::from_ssz_bytes(&serialized).unwrap();

        assert_eq!(input.block(), decoded.block());
        assert_eq!(input.aux_data(), decoded.aux_data());
    }
}
