use strata_db_types::l1_broadcast::L1TxEntry;
use strata_primitives::buf::Buf32;

use crate::{define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec};

define_table_without_codec!(
    /// A table to store mapping of idx to L1 txid
    (BcastL1TxIdSchema) u64 => Buf32
);
// `u64` keys use typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(BcastL1TxIdSchema, Buf32);

define_table_without_codec!(
    /// A table to store L1 txs
    (BcastL1TxSchema) Buf32 => L1TxEntry
);
impl_codec_key_codec!(BcastL1TxSchema, Buf32);
impl_cbor_value_codec!(BcastL1TxSchema, L1TxEntry);
