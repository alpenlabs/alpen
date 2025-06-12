use std::sync::Arc;

use strata_db::{traits::BroadcastDatabase, types::L1TxEntry};
use strata_rocksdb::{
    broadcaster::db::BroadcastDb, test_utils::get_rocksdb_tmp_instance, L1BroadcastDb, RBL1WriterDb,
};
use strata_storage::ops::{
    l1tx_broadcast::Context as BContext,
    writer::{Context, EnvelopeDataOps},
};

use crate::broadcaster::L1BroadcastHandle;

/// Returns [`Arc`] of [`RBL1WriterDb`] for testing
pub(crate) fn get_db() -> Arc<RBL1WriterDb> {
    let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
    Arc::new(RBL1WriterDb::new(db, db_ops))
}

/// Returns [`Arc`] of [`EnvelopeDataOps`] for testing
pub(crate) fn get_envelope_ops() -> Arc<EnvelopeDataOps> {
    let pool = threadpool::Builder::new().num_threads(2).build();
    let db = get_db();
    let ops = Context::new(db).into_ops(pool);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`BroadcastDatabase`] for testing
pub(crate) fn get_broadcast_db() -> Arc<impl BroadcastDatabase> {
    let (db, dbops) = get_rocksdb_tmp_instance().unwrap();
    let bcastdb = Arc::new(L1BroadcastDb::new(db, dbops));
    Arc::new(BroadcastDb::new(bcastdb))
}

/// Returns [`Arc`] of [`L1BroadcastHandle`] for testing
pub(crate) fn get_broadcast_handle() -> Arc<L1BroadcastHandle> {
    let pool = threadpool::Builder::new().num_threads(2).build();
    let db = get_broadcast_db();
    let ops = BContext::new(db.l1_broadcast_db().clone()).into_ops(pool);
    let (sender, _) = tokio::sync::mpsc::channel::<(u64, L1TxEntry)>(64);
    let handle = L1BroadcastHandle::new(sender, Arc::new(ops));
    Arc::new(handle)
}
