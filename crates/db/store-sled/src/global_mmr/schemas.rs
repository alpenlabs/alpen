use strata_db_types::mmr_helpers::MmrMetadata;
use strata_identifiers::RawMmrId;
use strata_primitives::buf::Buf32;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// Global MMR node storage: (mmr_id, position) -> hash
    (GlobalMmrNodeSchema) (RawMmrId, u64) => Buf32
);

define_table_with_seek_key_codec!(
    /// Global MMR metadata schema: mmr_id -> metadata
    (GlobalMmrMetaSchema) RawMmrId => MmrMetadata
);

define_table_with_seek_key_codec!(
    /// Global MMR hash position: (mmr_id, hash) -> position
    /// Enables reverse lookup from node hash to node position
    (GlobalMmrHashIndexSchema) (RawMmrId, Buf32) => u64
);

define_table_with_seek_key_codec!(
    /// Pre-image data storage: (mmr_id, leaf_index) -> serialized data
    /// Stores the actual data that was hashed to produce MMR leaves
    (GlobalMmrPreimageSchema) (RawMmrId, u64) => Vec<u8>
);
