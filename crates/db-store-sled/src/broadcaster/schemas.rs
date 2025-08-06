use strata_db::types::L1TxEntry;
use strata_primitives::buf::Buf32;

use crate::{
    define_table_with_default_codec, define_table_with_seek_key_codec, define_table_without_codec,
    impl_borsh_value_codec,
};

define_table_with_seek_key_codec!(
    /// A table to store mapping of idx to L1 txid
    (BcastL1TxIdSchema) u64 => Buf32
);

define_table_with_default_codec!(
    /// A table to store L1 txs
    (BcastL1TxSchema) Buf32 => L1TxEntry
);
