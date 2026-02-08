use strata_db_types::types::ChunkedEnvelopeEntry;

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_value_codec};

define_table_with_integer_key!(
    /// Stores idx -> chunked envelope entry mapping.
    (ChunkedEnvelopeSchema) u64 => ChunkedEnvelopeEntry
);
