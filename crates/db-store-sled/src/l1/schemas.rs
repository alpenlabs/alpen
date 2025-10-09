use strata_btc_types::legacy::{L1BlockManifest, L1Tx};
use strata_identifiers::L1BlockId;

use crate::{
    define_table_with_default_codec, define_table_with_integer_key, define_table_without_codec,
    impl_borsh_value_codec,
};

define_table_with_default_codec!(
    /// A table to store L1 Block data. Maps block id to header
    (L1BlockSchema) L1BlockId => L1BlockManifest
);

define_table_with_integer_key!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) u64 => L1BlockId
);

define_table_with_integer_key!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) u64 => Vec<L1BlockId>
);

define_table_with_default_codec!(
    /// A table to store L1 Txn data, maps block header hash to txns
    (TxnSchema) L1BlockId => Vec<L1Tx>
);
