use strata_db_types::mmr_helpers::{MmrId, MmrMetadata};
use strata_primitives::buf::Buf32;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// Unified MMR node storage: (mmr_id, position) -> hash
    (UnifiedMmrNodeSchema) (MmrId, u64) => Buf32
);

define_table_with_seek_key_codec!(
    /// Unified MMR metadata schema: mmr_id -> metadata
    (UnifiedMmrMetaSchema) MmrId => MmrMetadata
);

define_table_with_seek_key_codec!(
    /// Unified MMR hash index: (mmr_id, hash) -> position
    /// Enables reverse lookup from hash to leaf position
    (UnifiedMmrHashIndexSchema) (MmrId, Buf32) => u64
);

define_table_with_seek_key_codec!(
    /// Pre-image data storage: (mmr_id, leaf_index) -> serialized data
    /// Stores the actual data that was hashed to produce MMR leaves
    (UnifiedMmrPreimageSchema) (MmrId, u64) => Vec<u8>
);
