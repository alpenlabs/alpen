use std::sync::Arc;

use strata_db::{
    traits::{DatabaseBackend, L1BroadcastDatabase},
    types::L1TxEntry,
};
use strata_db_store_sled::{
    test_utils::{get_test_sled_backend, get_test_sled_config, get_test_sled_db},
    L1BroadcastDBSled,
};
use strata_storage::ops::{
    l1tx_broadcast::Context as BContext,
    writer::{Context, EnvelopeDataOps},
};

use crate::broadcaster::L1BroadcastHandle;

/// Returns [`Arc`] of [`EnvelopeDataOps`] for testing
pub(crate) fn get_envelope_ops() -> Arc<EnvelopeDataOps> {
    let pool = threadpool::Builder::new().num_threads(2).build();
    let db = get_test_sled_backend().writer_db();
    let ops = Context::new(db).into_ops(pool);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`BroadcastDatabase`] for testing
pub(crate) fn get_broadcast_db() -> Arc<impl L1BroadcastDatabase> {
    let sdb = get_test_sled_db();
    let sconf = get_test_sled_config();
    Arc::new(L1BroadcastDBSled::new(sdb.into(), sconf).unwrap())
}

/// Returns [`Arc`] of [`L1BroadcastHandle`] for testing
pub(crate) fn get_broadcast_handle() -> Arc<L1BroadcastHandle> {
    let pool = threadpool::Builder::new().num_threads(2).build();
    let db = get_broadcast_db();
    let ops = BContext::new(db).into_ops(pool);
    let (sender, _) = tokio::sync::mpsc::channel::<(u64, L1TxEntry)>(64);
    let handle = L1BroadcastHandle::new(sender, Arc::new(ops));
    Arc::new(handle)
}
