use bitcoin::{
    Block,
    consensus::{deserialize, serialize},
    hashes::Hash,
};
use moho_types::StateReference;
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
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
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct L1Block(#[rkyv(with = BlockAsBytes)] pub Block);

/// Serializer for [`Block`] as bytes for rkyv.
struct BlockAsBytes;

impl ArchiveWith<Block> for BlockAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &Block, resolver: Self::Resolver, out: Place<Self::Archived>) {
        let bytes = serialize(field);
        rkyv::Archive::resolve(&bytes, resolver, out);
    }
}

impl<S> SerializeWith<Block, S> for BlockAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(field: &Block, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        let bytes = serialize(field);
        rkyv::Serialize::serialize(&bytes, serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, Block, D> for BlockAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<Block, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(deserialize(&bytes).expect("stored block should decode"))
    }
}
