use bitcoin::{
    Block,
    consensus::{deserialize, serialize},
    hashes::Hash,
};
use moho_types::StateReference;
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_asm_common::AuxData;

/// Private input to process the next state.
///
/// This includes all the L1
#[derive(Clone, Debug, DeriveEncode, DeriveDecode)]
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

/// A wrapper around Bitcoin's `Block` to provide Borsh (de)serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct L1Block(pub Block);

impl Encode for L1Block {
    fn is_ssz_fixed_len() -> bool {
        <Vec<u8> as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <Vec<u8> as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        serialize(&self.0).ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        serialize(&self.0).ssz_bytes_len()
    }
}

impl Decode for L1Block {
    fn is_ssz_fixed_len() -> bool {
        <Vec<u8> as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <Vec<u8> as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let encoded = Vec::<u8>::from_ssz_bytes(bytes)?;
        let block =
            deserialize(&encoded).map_err(|err| DecodeError::BytesInvalid(err.to_string()))?;
        Ok(Self(block))
    }
}

#[cfg(test)]
mod tests {

    use strata_test_utils_btc::segment::BtcChainSegment;

    use super::*;

    #[test]
    fn test_ssz_roundtrip() {
        let block = BtcChainSegment::load_full_block();
        let l1_block = L1Block(block);

        let ssz_serialized = l1_block.as_ssz_bytes();
        let ssz_deserialized = L1Block::from_ssz_bytes(&ssz_serialized).unwrap();

        assert_eq!(l1_block, ssz_deserialized);
    }
}
