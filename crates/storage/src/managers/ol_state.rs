//! High-level OL state interface.

use std::{future::Future, num::NonZeroUsize, sync::Arc};

use futures::TryFutureExt;
use strata_db_types::{errors::DbError, traits::OLStateDatabase, DbResult};
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::{OLAccountState, OLState, StateProvider, WriteBatch};
use strata_storage_common::exec::{GenericRecv, OpsError};
use threadpool::ThreadPool;
use tokio::sync::oneshot;

use crate::{
    cache::CacheTable,
    ops::ol_state::{Context, OLStateOps},
};

/// Default cache capacity for OL state and write batch caches.
const DEFAULT_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(64).expect("64 is non-zero");

/// Helper to transform a channel receiver from `Option<OLState>` to `Option<Arc<OLState>>`.
fn transform_ol_state_chan(
    rx: GenericRecv<Option<OLState>, DbError>,
) -> GenericRecv<Option<Arc<OLState>>, DbError> {
    let (tx, new_rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = match rx.await {
            Ok(Ok(opt)) => Ok(opt.map(Arc::new)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(OpsError::WorkerFailedStrangely.into()),
        };
        let _ = tx.send(result);
    });
    new_rx
}

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLStateManager {
    ops: OLStateOps,
    state_cache: CacheTable<OLBlockCommitment, Option<Arc<OLState>>>,
    wb_cache: CacheTable<OLBlockCommitment, Option<WriteBatch<OLAccountState>>>,
}

impl OLStateManager {
    pub fn new<D: OLStateDatabase + Sync + Send + 'static>(pool: ThreadPool, db: Arc<D>) -> Self {
        let ops = Context::new(db.clone()).into_ops(pool);
        let state_cache = CacheTable::new(DEFAULT_CACHE_CAPACITY);
        let wb_cache = CacheTable::new(DEFAULT_CACHE_CAPACITY);
        Self {
            ops,
            state_cache,
            wb_cache,
        }
    }

    /// Stores a toplevel OLState snapshot for a given block commitment.
    pub async fn put_toplevel_ol_state_async(
        &self,
        commitment: OLBlockCommitment,
        state: OLState,
    ) -> DbResult<()> {
        self.ops
            .put_toplevel_ol_state_async(commitment, state.clone())
            .await?;
        self.state_cache
            .insert_async(commitment, Some(Arc::new(state)))
            .await;
        Ok(())
    }

    /// Stores a toplevel OLState snapshot for a given block commitment.
    pub fn put_toplevel_ol_state_blocking(
        &self,
        commitment: OLBlockCommitment,
        state: OLState,
    ) -> DbResult<()> {
        self.ops
            .put_toplevel_ol_state_blocking(commitment, state.clone())?;
        self.state_cache
            .insert_blocking(commitment, Some(Arc::new(state)));
        Ok(())
    }

    /// Retrieves a toplevel OLState snapshot for a given block commitment.
    pub async fn get_toplevel_ol_state_async(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>> {
        self.state_cache
            .get_or_fetch(&commitment, || {
                transform_ol_state_chan(self.ops.get_toplevel_ol_state_chan(commitment))
            })
            .await
    }

    /// Retrieves a toplevel OLState snapshot for a given block commitment.
    pub fn get_toplevel_ol_state_blocking(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>> {
        self.state_cache.get_or_fetch_blocking(&commitment, || {
            self.ops
                .get_toplevel_ol_state_blocking(commitment)
                .map(|opt| opt.map(Arc::new))
        })
    }

    /// Gets the latest toplevel OLState (highest slot).
    pub async fn get_latest_toplevel_ol_state_async(
        &self,
    ) -> DbResult<Option<(OLBlockCommitment, Arc<OLState>)>> {
        self.ops
            .get_latest_toplevel_ol_state_async()
            .map_ok(|opt| opt.map(|(c, s)| (c, Arc::new(s))))
            .await
    }

    /// Gets the latest toplevel OLState (highest slot).
    pub fn get_latest_toplevel_ol_state_blocking(
        &self,
    ) -> DbResult<Option<(OLBlockCommitment, Arc<OLState>)>> {
        self.ops
            .get_latest_toplevel_ol_state_blocking()
            .map(|opt| opt.map(|(c, s)| (c, Arc::new(s))))
    }

    /// Deletes a toplevel OLState snapshot for a given block commitment.
    pub async fn del_toplevel_ol_state_async(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_toplevel_ol_state_async(commitment).await?;
        self.state_cache.purge_async(&commitment).await;
        Ok(())
    }

    /// Deletes a toplevel OLState snapshot for a given block commitment.
    pub fn del_toplevel_ol_state_blocking(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_toplevel_ol_state_blocking(commitment)?;
        self.state_cache.purge_blocking(&commitment);
        Ok(())
    }

    /// Stores a write batch for a given block commitment.
    pub async fn put_write_batch_async(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountState>,
    ) -> DbResult<()> {
        self.ops
            .put_ol_write_batch_async(commitment, wb.clone())
            .await?;
        self.wb_cache.insert_async(commitment, Some(wb)).await;
        Ok(())
    }

    /// Stores a write batch for a given block commitment.
    pub fn put_write_batch_blocking(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountState>,
    ) -> DbResult<()> {
        self.ops
            .put_ol_write_batch_blocking(commitment, wb.clone())?;
        self.wb_cache.insert_blocking(commitment, Some(wb));
        Ok(())
    }

    /// Retrieves a write batch for a given block commitment.
    pub async fn get_write_batch_async(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountState>>> {
        self.wb_cache
            .get_or_fetch(&commitment, || self.ops.get_ol_write_batch_chan(commitment))
            .await
    }

    /// Retrieves a write batch for a given block commitment.
    pub fn get_write_batch_blocking(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountState>>> {
        self.wb_cache.get_or_fetch_blocking(&commitment, || {
            self.ops.get_ol_write_batch_blocking(commitment)
        })
    }

    /// Deletes a write batch for a given block commitment.
    pub async fn del_write_batch_async(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_ol_write_batch_async(commitment).await?;
        self.wb_cache.purge_async(&commitment).await;
        Ok(())
    }

    /// Deletes a write batch for a given block commitment.
    pub fn del_write_batch_blocking(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_ol_write_batch_blocking(commitment)?;
        self.wb_cache.purge_blocking(&commitment);
        Ok(())
    }
}

// Implement StateProvider trait for OLStateManager
impl StateProvider for OLStateManager {
    type State = OLState;
    type Error = DbError;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Arc<Self::State>>, Self::Error>> + Send {
        self.get_toplevel_ol_state_async(tip)
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Arc<Self::State>>, Self::Error> {
        self.get_toplevel_ol_state_blocking(tip)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::DatabaseBackend;
    use strata_identifiers::{OLBlockCommitment, OLBlockId, Slot};
    use strata_ledger_types::IStateAccessor;
    use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
    use threadpool::ThreadPool;

    use super::*;

    fn setup_manager() -> OLStateManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let ol_state_db = db.ol_state_db();
        OLStateManager::new(pool, ol_state_db)
    }

    #[tokio::test]
    async fn test_put_and_get_ol_state_async() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put state
        manager
            .put_toplevel_ol_state_async(commitment, state.clone())
            .await
            .expect("test: put");

        // Get state (should be cached)
        let retrieved = manager
            .get_toplevel_ol_state_async(commitment)
            .await
            .expect("test: get")
            .unwrap();
        // Verify state was retrieved (can't compare directly as OLState doesn't implement
        // PartialEq)
        assert_eq!(retrieved.cur_slot(), state.cur_slot());
    }

    #[test]
    fn test_put_and_get_ol_state_blocking() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put state
        manager
            .put_toplevel_ol_state_blocking(commitment, state.clone())
            .expect("test: put");

        // Get state (should be cached)
        let retrieved = manager
            .get_toplevel_ol_state_blocking(commitment)
            .expect("test: get")
            .unwrap();
        // Verify state was retrieved (can't compare directly as OLState doesn't implement
        // PartialEq)
        assert_eq!(retrieved.cur_slot(), state.cur_slot());
    }

    #[tokio::test]
    async fn test_get_latest_ol_state_async() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment1 = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());
        let commitment2 = OLBlockCommitment::new(Slot::from(1u64), OLBlockId::default());

        // Put two states
        manager
            .put_toplevel_ol_state_async(commitment1, state.clone())
            .await
            .expect("test: put 1");
        manager
            .put_toplevel_ol_state_async(commitment2, state.clone())
            .await
            .expect("test: put 2");

        // Latest should be the one with highest slot
        let (latest_commitment, latest_state) = manager
            .get_latest_toplevel_ol_state_async()
            .await
            .expect("test: get latest")
            .unwrap();
        assert_eq!(latest_commitment, commitment2);
        assert_eq!(latest_state.cur_slot(), state.cur_slot());
    }

    #[test]
    fn test_get_latest_ol_state_blocking() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment1 = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());
        let commitment2 = OLBlockCommitment::new(Slot::from(1u64), OLBlockId::default());

        // Put two states
        manager
            .put_toplevel_ol_state_blocking(commitment1, state.clone())
            .expect("test: put 1");
        manager
            .put_toplevel_ol_state_blocking(commitment2, state.clone())
            .expect("test: put 2");

        // Latest should be the one with highest slot
        let (latest_commitment, latest_state) = manager
            .get_latest_toplevel_ol_state_blocking()
            .expect("test: get latest")
            .unwrap();
        assert_eq!(latest_commitment, commitment2);
        assert_eq!(latest_state.cur_slot(), state.cur_slot());
    }

    #[tokio::test]
    async fn test_delete_ol_state_async() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put state
        manager
            .put_toplevel_ol_state_async(commitment, state)
            .await
            .expect("test: put");

        // Delete state
        manager
            .del_toplevel_ol_state_async(commitment)
            .await
            .expect("test: delete");

        // Verify it's gone
        let deleted = manager
            .get_toplevel_ol_state_async(commitment)
            .await
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    #[test]
    fn test_delete_ol_state_blocking() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put state
        manager
            .put_toplevel_ol_state_blocking(commitment, state)
            .expect("test: put");

        // Delete state
        manager
            .del_toplevel_ol_state_blocking(commitment)
            .expect("test: delete");

        // Verify it's gone
        let deleted = manager
            .get_toplevel_ol_state_blocking(commitment)
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_write_batch_operations_async() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put write batch
        manager
            .put_write_batch_async(commitment, wb.clone())
            .await
            .expect("test: put");

        // Get write batch (should be cached)
        let retrieved = manager
            .get_write_batch_async(commitment)
            .await
            .expect("test: get")
            .unwrap();
        // Verify write batch was retrieved (can't compare directly as WriteBatch doesn't implement
        // PartialEq)
        assert_eq!(
            retrieved.global().get_cur_slot(),
            wb.global().get_cur_slot()
        );

        // Delete write batch
        manager
            .del_write_batch_async(commitment)
            .await
            .expect("test: delete");

        // Verify it's gone
        let deleted = manager
            .get_write_batch_async(commitment)
            .await
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    #[test]
    fn test_write_batch_operations_blocking() {
        let manager = setup_manager();
        let state = OLState::new_genesis();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        let commitment = OLBlockCommitment::new(Slot::from(0u64), OLBlockId::default());

        // Put write batch
        manager
            .put_write_batch_blocking(commitment, wb.clone())
            .expect("test: put");

        // Get write batch (should be cached)
        let retrieved = manager
            .get_write_batch_blocking(commitment)
            .expect("test: get")
            .unwrap();
        // Verify write batch was retrieved (can't compare directly as WriteBatch doesn't implement
        // PartialEq)
        assert_eq!(
            retrieved.global().get_cur_slot(),
            wb.global().get_cur_slot()
        );

        // Delete write batch
        manager
            .del_write_batch_blocking(commitment)
            .expect("test: delete");

        // Verify it's gone
        let deleted = manager
            .get_write_batch_blocking(commitment)
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }
}
