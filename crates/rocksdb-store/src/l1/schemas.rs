use strata_mmr::CompactMmr;
use strata_primitives::l1::{L1Block, L1BlockId, L1BlockManifest, L1Tx};

use crate::{
    define_table_with_default_codec, define_table_with_seek_key_codec, define_table_without_codec,
    impl_borsh_value_codec,
};

// L1 Block Schema and corresponding codecs implementation
define_table_with_default_codec!(
    /// A table to store L1 Block data. Maps block id to header
    (L1BlockSchema) L1BlockId => L1BlockManifest
);

define_table_with_default_codec!(
    /// A table to store L1 Block data. Maps block id to header
    (L1RawBlockSchema) L1BlockId => L1Block
);

define_table_with_seek_key_codec!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) u64 => L1BlockId
);

define_table_with_seek_key_codec!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) u64 => Vec<L1BlockId>
);

// L1 Txns Schema and corresponding codecs implementation
define_table_with_default_codec!(
    /// A table to store L1 Txn data, maps block header hash to txns
    (TxnSchema) L1BlockId => Vec<L1Tx>
);
