use strata_db_types::{LeafPos, NodePos, RawMmrId};
use strata_primitives::buf::Buf32;

use crate::define_table_with_seek_key_codec;

define_table_with_seek_key_codec!(
    /// MMR index node storage: (mmr_id, node_pos) -> hash
    (MmrIndexNodeSchema) (RawMmrId, NodePos) => Buf32
);

define_table_with_seek_key_codec!(
    /// MMR index preimage storage: (mmr_id, leaf_pos) -> preimage bytes
    (MmrIndexPreimageSchema) (RawMmrId, LeafPos) => Vec<u8>
);

define_table_with_seek_key_codec!(
    /// MMR index leaf count storage: mmr_id -> leaf count
    (MmrIndexLeafCountSchema) RawMmrId => u64
);
