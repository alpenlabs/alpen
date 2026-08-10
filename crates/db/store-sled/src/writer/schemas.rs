use strata_db_types::l1_writer::{BundledPayloadEntry, IntentEntry};
use strata_primitives::buf::Buf32;

use crate::{define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec};

define_table_without_codec!(
    /// A table to store idx-> payload entry mapping
    (PayloadSchema) u64 => BundledPayloadEntry
);
// `u64` keys use typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(PayloadSchema, BundledPayloadEntry);

define_table_without_codec!(
    /// A table to store intentid -> intent mapping
    (IntentSchema) Buf32 => IntentEntry
);
impl_codec_key_codec!(IntentSchema, Buf32);
impl_cbor_value_codec!(IntentSchema, IntentEntry);

define_table_without_codec!(
    /// A table to store idx-> intent id mapping
    (IntentIdxSchema) u64 => Buf32
);
impl_cbor_value_codec!(IntentIdxSchema, Buf32);
