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

    use proptest::prelude::*;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::DatabaseBackend;
    use strata_identifiers::{test_utils::ol_block_commitment_strategy, OLBlockCommitment};
    use strata_ledger_types::IStateAccessor;
    use strata_ol_state_types::{
        test_utils::ol_state_strategy, OLAccountState, OLState, WriteBatch,
    };
    use threadpool::ThreadPool;
    use tokio::runtime::Runtime;

    use super::*;

    fn setup_manager() -> OLStateManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let ol_state_db = db.ol_state_db();
        OLStateManager::new(pool, ol_state_db)
    }

    // =============================================================================
    // Proptest helper functions (blocking)
    // =============================================================================

    fn proptest_put_and_get_toplevel_blocking(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        manager
            .put_toplevel_ol_state_blocking(commitment, state.clone())
            .expect("test: put");
        let retrieved = manager
            .get_toplevel_ol_state_blocking(commitment)
            .expect("test: get")
            .unwrap();
        assert_eq!(retrieved.cur_slot(), state.cur_slot());
    }

    fn proptest_get_latest_toplevel_blocking(
        commitment1: OLBlockCommitment,
        commitment2: OLBlockCommitment,
        state: OLState,
    ) {
        let manager = setup_manager();
        let (lower, higher) = if commitment1.slot() < commitment2.slot() {
            (commitment1, commitment2)
        } else if commitment1.slot() > commitment2.slot() {
            (commitment2, commitment1)
        } else if commitment1.blkid() < commitment2.blkid() {
            (commitment1, commitment2)
        } else {
            (commitment2, commitment1)
        };
        manager
            .put_toplevel_ol_state_blocking(lower, state.clone())
            .expect("test: put 1");
        manager
            .put_toplevel_ol_state_blocking(higher, state.clone())
            .expect("test: put 2");
        let (latest_commitment, latest_state) = manager
            .get_latest_toplevel_ol_state_blocking()
            .expect("test: get latest")
            .unwrap();
        assert_eq!(latest_commitment, higher);
        assert_eq!(latest_state.cur_slot(), state.cur_slot());
    }

    fn proptest_delete_toplevel_blocking(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        manager
            .put_toplevel_ol_state_blocking(commitment, state)
            .expect("test: put");
        manager
            .del_toplevel_ol_state_blocking(commitment)
            .expect("test: delete");
        let deleted = manager
            .get_toplevel_ol_state_blocking(commitment)
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    fn proptest_put_and_get_write_batch_blocking(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        manager
            .put_write_batch_blocking(commitment, wb.clone())
            .expect("test: put");
        let retrieved = manager
            .get_write_batch_blocking(commitment)
            .expect("test: get")
            .unwrap();
        assert_eq!(
            retrieved.global().get_cur_slot(),
            wb.global().get_cur_slot()
        );
    }

    fn proptest_delete_write_batch_blocking(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        manager
            .put_write_batch_blocking(commitment, wb)
            .expect("test: put");
        manager
            .del_write_batch_blocking(commitment)
            .expect("test: delete");
        let deleted = manager
            .get_write_batch_blocking(commitment)
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    // =============================================================================
    // Proptest helper functions (async)
    // =============================================================================

    async fn proptest_put_and_get_toplevel_async(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        manager
            .put_toplevel_ol_state_async(commitment, state.clone())
            .await
            .expect("test: put");
        let retrieved = manager
            .get_toplevel_ol_state_async(commitment)
            .await
            .expect("test: get")
            .unwrap();
        assert_eq!(retrieved.cur_slot(), state.cur_slot());
    }

    async fn proptest_get_latest_toplevel_async(
        commitment1: OLBlockCommitment,
        commitment2: OLBlockCommitment,
        state: OLState,
    ) {
        let manager = setup_manager();
        let (lower, higher) = if commitment1.slot() < commitment2.slot() {
            (commitment1, commitment2)
        } else if commitment1.slot() > commitment2.slot() {
            (commitment2, commitment1)
        } else if commitment1.blkid() < commitment2.blkid() {
            (commitment1, commitment2)
        } else {
            (commitment2, commitment1)
        };
        manager
            .put_toplevel_ol_state_async(lower, state.clone())
            .await
            .expect("test: put 1");
        manager
            .put_toplevel_ol_state_async(higher, state.clone())
            .await
            .expect("test: put 2");
        let (latest_commitment, latest_state) = manager
            .get_latest_toplevel_ol_state_async()
            .await
            .expect("test: get latest")
            .unwrap();
        assert_eq!(latest_commitment, higher);
        assert_eq!(latest_state.cur_slot(), state.cur_slot());
    }

    async fn proptest_delete_toplevel_async(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        manager
            .put_toplevel_ol_state_async(commitment, state)
            .await
            .expect("test: put");
        manager
            .del_toplevel_ol_state_async(commitment)
            .await
            .expect("test: delete");
        let deleted = manager
            .get_toplevel_ol_state_async(commitment)
            .await
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    async fn proptest_put_and_get_write_batch_async(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        manager
            .put_write_batch_async(commitment, wb.clone())
            .await
            .expect("test: put");
        let retrieved = manager
            .get_write_batch_async(commitment)
            .await
            .expect("test: get")
            .unwrap();
        assert_eq!(
            retrieved.global().get_cur_slot(),
            wb.global().get_cur_slot()
        );
    }

    async fn proptest_delete_write_batch_async(commitment: OLBlockCommitment, state: OLState) {
        let manager = setup_manager();
        let wb = WriteBatch::<OLAccountState>::new_from_state(&state);
        manager
            .put_write_batch_async(commitment, wb)
            .await
            .expect("test: put");
        manager
            .del_write_batch_async(commitment)
            .await
            .expect("test: delete");
        let deleted = manager
            .get_write_batch_async(commitment)
            .await
            .expect("test: get after delete");
        assert!(deleted.is_none());
    }

    // =============================================================================
    // Proptest-based tests (blocking)
    // =============================================================================

    proptest! {
        #[test]
        fn test_put_and_get_toplevel_blocking(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            proptest_put_and_get_toplevel_blocking(commitment, state);
        }

        #[test]
        fn test_get_latest_toplevel_blocking(
            commitment1 in ol_block_commitment_strategy(),
            commitment2 in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            proptest_get_latest_toplevel_blocking(commitment1, commitment2, state);
        }

        #[test]
        fn test_delete_toplevel_blocking(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            proptest_delete_toplevel_blocking(commitment, state);
        }

        #[test]
        fn test_put_and_get_write_batch_blocking(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            proptest_put_and_get_write_batch_blocking(commitment, state);
        }

        #[test]
        fn test_delete_write_batch_blocking(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            proptest_delete_write_batch_blocking(commitment, state);
        }
    }

    // =============================================================================
    // Proptest-based tests (async)
    // =============================================================================

    proptest! {
        #[test]
        fn test_put_and_get_toplevel_async(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            Runtime::new().unwrap().block_on(async {
                proptest_put_and_get_toplevel_async(commitment, state).await;
            });
        }

        #[test]
        fn test_get_latest_toplevel_async(
            commitment1 in ol_block_commitment_strategy(),
            commitment2 in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            Runtime::new().unwrap().block_on(async {
                proptest_get_latest_toplevel_async(commitment1, commitment2, state).await;
            });
        }

        #[test]
        fn test_delete_toplevel_async(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            Runtime::new().unwrap().block_on(async {
                proptest_delete_toplevel_async(commitment, state).await;
            });
        }

        #[test]
        fn test_put_and_get_write_batch_async(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            Runtime::new().unwrap().block_on(async {
                proptest_put_and_get_write_batch_async(commitment, state).await;
            });
        }

        #[test]
        fn test_delete_write_batch_async(
            commitment in ol_block_commitment_strategy(),
            state in ol_state_strategy(),
        ) {
            Runtime::new().unwrap().block_on(async {
                proptest_delete_write_batch_async(commitment, state).await;
            });
        }
    }
}
