//! Operations for reading/writing envelope related data from/to Database

use std::sync::Arc;

use strata_db::{
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry},
    DbResult,
};
use strata_identifiers::Buf32;

use crate::exec::*;

inst_ops_simple! {
    (<D: L1WriterDatabase> => EnvelopeDataOps) {
        put_payload_entry(idx: u64, payloadentry: BundledPayloadEntry) => ();
        get_payload_entry_by_idx(idx: u64) => Option<BundledPayloadEntry>;
        get_next_payload_idx() => u64;
        put_intent_entry(id: Buf32, entry: IntentEntry) => ();
        get_intent_by_id(id: Buf32) => Option<IntentEntry>;
        get_intent_by_idx(idx: u64) => Option<IntentEntry>;
        get_next_intent_idx() => u64;
    }
}
