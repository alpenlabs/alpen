use std::sync::Arc;

use strata_db_store_sled::{
    test_utils::{get_test_sled_backend, get_test_sled_config, get_test_sled_db},
    SledBackend,
};
use strata_db_types::backend::DatabaseBackend;
use strata_storage::{
    ops::{chunked_envelope::ChunkedEnvelopeOps, writer::EnvelopeDataOps},
    BroadcastDbOps,
};
use tokio::runtime::Handle;

use crate::broadcaster::L1BroadcastHandle;

/// Returns [`Arc`] of [`EnvelopeDataOps`] for testing
pub(crate) fn get_envelope_ops() -> Arc<EnvelopeDataOps> {
    let db = get_test_sled_backend().writer_db();
    let ops = EnvelopeDataOps::new(Handle::current(), db);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`ChunkedEnvelopeOps`] for testing.
pub(crate) fn get_chunked_envelope_ops() -> Arc<ChunkedEnvelopeOps> {
    let db = get_test_sled_backend().chunked_envelope_db();
    let ops = ChunkedEnvelopeOps::new(Handle::current(), db);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`L1BroadcastHandle`] for testing
pub(crate) fn get_broadcast_handle() -> Arc<L1BroadcastHandle> {
    let sdb = get_test_sled_db();
    let sconf = get_test_sled_config();
    let backend = SledBackend::new(sdb.into(), sconf).unwrap();
    let db = backend.broadcast_db();
    let ops = BroadcastDbOps::new(Handle::current(), db);
    let handle = L1BroadcastHandle::new_for_test(Arc::new(ops));
    Arc::new(handle)
}
