use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_primitives::l1::L1BlockCommitment;

use crate::{
    define_table_with_integer_key, define_table_with_seek_key_codec, define_table_without_codec,
    impl_borsh_value_codec,
};

// ASM state per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);

// ASM logs per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);

// MMR database schemas for aux data resolution

define_table_with_integer_key!(
    /// MMR node storage: position -> hash. Stores all MMR nodes for proof generation.
    (AsmMmrNodeSchema) u64 => [u8; 32]
);

/// MMR metadata storage
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct MmrMetadata {
    pub num_leaves: u64,
    pub mmr_size: u64,
    pub peak_roots: Vec<[u8; 32]>,
}

define_table_without_codec!(
    /// A table to store MMR metadata (singleton)
    (AsmMmrMetaSchema) () => MmrMetadata
);

// Implement KeyCodec for unit type (singleton key)
impl ::typed_sled::codec::KeyCodec<AsmMmrMetaSchema> for () {
    fn encode_key(&self) -> Result<Vec<u8>, ::typed_sled::codec::CodecError> {
        Ok(vec![0u8]) // Single byte for singleton key
    }

    fn decode_key(bytes: &[u8]) -> Result<Self, ::typed_sled::codec::CodecError> {
        if bytes.len() == 1 && bytes[0] == 0 {
            Ok(())
        } else {
            Err(::typed_sled::codec::CodecError::InvalidKeyLength {
                schema: "AsmMmrMetaSchema",
                expected: 1,
                actual: bytes.len(),
            })
        }
    }
}

impl_borsh_value_codec!(AsmMmrMetaSchema, MmrMetadata);

define_table_with_integer_key!(
    /// Manifest hash storage: manifest_index -> hash. Maps leaf indices to manifest hashes.
    (AsmManifestHashSchema) u64 => [u8; 32]
);
