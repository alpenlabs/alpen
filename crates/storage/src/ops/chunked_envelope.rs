//! Operations for reading/writing chunked envelope data from/to database.

use strata_db_types::{traits::L1ChunkedEnvelopeDatabase, types::ChunkedEnvelopeEntry};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: L1ChunkedEnvelopeDatabase> => ChunkedEnvelopeOps, component = components::STORAGE_CHUNKED_ENVELOPE) {
        put_chunked_envelope_entry(idx: u64, entry: ChunkedEnvelopeEntry) => ();
        get_chunked_envelope_entry(idx: u64) => Option<ChunkedEnvelopeEntry>;
        get_next_chunked_envelope_idx() => u64;
        del_chunked_envelope_entry(idx: u64) => bool;
        del_chunked_envelope_entries_from_idx(start_idx: u64) => Vec<u64>;
    }
}
