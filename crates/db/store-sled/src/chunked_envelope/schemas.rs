use strata_db_types::chunked_envelope::ChunkedEnvelopeEntry;

use crate::define_table_with_integer_key;

define_table_with_integer_key!(
    /// Stores idx -> chunked envelope entry mapping.
    (ChunkedEnvelopeSchema) u64 => ChunkedEnvelopeEntry
);
