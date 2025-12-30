use strata_db_types::mmr_helpers::MmrMetadata;
use strata_primitives::buf::Buf32;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// Unified MMR node storage: (mmr_id, position) -> hash
    (UnifiedMmrNodeSchema) (Vec<u8>, u64) => Buf32
);

define_table_with_seek_key_codec!(
    /// Unified MMR metadata schema: mmr_id -> metadata
    (UnifiedMmrMetaSchema) Vec<u8> => MmrMetadata
);

define_table_with_seek_key_codec!(
    /// Unified MMR hash position: (mmr_id, hash) -> position
    /// Enables reverse lookup from node hash to node position
    (UnifiedMmrHashIndexSchema) (Vec<u8>, Buf32) => u64
);

define_table_with_seek_key_codec!(
    /// Pre-image data storage: (mmr_id, leaf_index) -> serialized data
    /// Stores the actual data that was hashed to produce MMR leaves
    (UnifiedMmrPreimageSchema) (Vec<u8>, u64) => Vec<u8>
);
