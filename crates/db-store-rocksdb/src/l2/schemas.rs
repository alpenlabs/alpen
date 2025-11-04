use strata_db::traits::BlockStatus;
use strata_ol_chain_types::{L2BlockBundle, L2BlockId};

use crate::{define_table_with_default_codec, define_table_with_seek_key_codec};

define_table_with_default_codec!(
    /// A table to store L2 Block data. Maps block id to Block
    (L2BlockSchema) L2BlockId => L2BlockBundle
);

define_table_with_default_codec!(
    /// A table to store L2 Block data. Maps block id to BlockStatus
    (L2BlockStatusSchema) L2BlockId => BlockStatus
);

define_table_with_seek_key_codec!(
    /// A table to store L2 Block data. Maps block id to BlockId
    (L2BlockHeightSchema) u64 => Vec<L2BlockId>
);
