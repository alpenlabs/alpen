use strata_db_types::chunked_envelope::ChunkedEnvelopeEntry;

use crate::{define_table_without_codec, impl_cbor_value_codec};

define_table_without_codec!(
    /// Stores idx -> chunked envelope entry mapping.
    (ChunkedEnvelopeSchema) u64 => ChunkedEnvelopeEntry
);
// `u64` keys use typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(ChunkedEnvelopeSchema, ChunkedEnvelopeEntry);
