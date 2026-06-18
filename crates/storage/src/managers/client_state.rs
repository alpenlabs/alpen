//! Client state manager.
// TODO(STR-3679): should this also include sync events?

use std::sync::Arc;

use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db_types::{client_state::ClientStateDatabase, DbResult};
use strata_primitives::{l1::L1BlockCommitment, L1Height};
use tokio::{runtime::Handle, sync::Mutex};

use crate::{cache, ops::client_state::ClientStateOps};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ClientStateManager {
    ops: ClientStateOps,

    // TODO(STR-3679): actually use caches
    update_cache: cache::CacheTable<L1Height, Option<ClientUpdateOutput>>,
    state_cache: cache::CacheTable<L1Height, Arc<ClientState>>,

    cur_state: Mutex<CurStateTracker>,
}

impl ClientStateManager {
    pub fn new(handle: Handle, db: Arc<impl ClientStateDatabase + 'static>) -> DbResult<Self> {
        let ops = ClientStateOps::new(handle, db);
        let update_cache = cache::CacheTable::new(64.try_into().unwrap());
        let state_cache = cache::CacheTable::new(64.try_into().unwrap());

        // Setup the tracker to point at the last or default pregenesis client state.
        let mut cur_state = CurStateTracker::new_empty();

        let latest_cs = ops.get_latest_client_state_blocking()?;
        if let Some((blk, cs)) = latest_cs {
            cur_state.set(blk.height(), Arc::new(cs));
        }

        Ok(Self {
            ops,
            update_cache,
            state_cache,
            cur_state: Mutex::new(cur_state),
        })
    }

    // TODO(STR-3679): convert to managing these with Arcs
    pub async fn get_state_async(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .ops
            .get_client_update_async(block)
            .await?
            .map(|update| update.into_state()))
    }

    pub fn get_state_blocking(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .ops
            .get_client_update_blocking(block)?
            .map(|update| update.into_state()))
    }

    pub fn get_update_blocking(
        &self,
        block: &L1BlockCommitment,
    ) -> DbResult<Option<ClientUpdateOutput>> {
        self.ops.get_client_update_blocking(*block)
    }

    pub fn put_update_blocking(
        &self,
        block: &L1BlockCommitment,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        // FIXME(STR-3679): this is a lot of cloning, good thing the type isn't gigantic,
        // still feels bad though
        let state = Arc::new(update.state().clone());
        let height = block.height();
        self.ops
            .put_client_update_blocking(*block, update.clone())?;
        self.maybe_update_cur_state_blocking(height, &state);
        self.update_cache.insert_blocking(height, Some(update));
        self.state_cache.insert_blocking(height, state.clone());
        Ok(state)
    }

    pub async fn put_update_async(
        &self,
        block: &L1BlockCommitment,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        // FIXME(STR-3679): this is a lot of cloning, good thing the type isn't gigantic,
        // still feels bad though
        let state = Arc::new(update.state().clone());
        let height = block.height();
        self.ops
            .put_client_update_async(*block, update.clone())
            .await?;
        self.maybe_update_cur_state_async(height, &state).await;
        self.update_cache.insert_async(height, Some(update)).await;
        self.state_cache.insert_async(height, state.clone()).await;
        Ok(state)
    }

    /// Deletes the client update at `block`, keeping caches and the current
    /// state tracker coherent.
    pub fn del_update_blocking(&self, block: &L1BlockCommitment) -> DbResult<()> {
        let height = block.height();
        self.ops.del_client_update_blocking(*block)?;
        self.update_cache.purge_blocking(&height);
        self.state_cache.purge_blocking(&height);
        self.reset_cur_state_blocking()?;
        Ok(())
    }

    fn maybe_update_cur_state_blocking(&self, height: L1Height, state: &Arc<ClientState>) -> bool {
        let mut cur = self.cur_state.blocking_lock();
        cur.maybe_update(height, state)
    }

    async fn maybe_update_cur_state_async(
        &self,
        height: L1Height,
        state: &Arc<ClientState>,
    ) -> bool {
        let mut cur = self.cur_state.lock().await;
        cur.maybe_update(height, state)
    }

    /// Recomputes the current-state tracker from storage after a deletion.
    fn reset_cur_state_blocking(&self) -> DbResult<()> {
        let mut cur = self.cur_state.blocking_lock();
        *cur = CurStateTracker::new_empty();
        if let Some((blk, cs)) = self.ops.get_latest_client_state_blocking()? {
            cur.set(blk.height(), Arc::new(cs));
        }
        Ok(())
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the greatest key.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        self.ops.get_latest_client_state_blocking()
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the greatest key.
    pub async fn fetch_most_recent_state_async(
        &self,
    ) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        self.ops.get_latest_client_state_async().await
    }

    /// Returns [`ClientUpdateOutput`] entries starting from a given block up to a maximum count.
    ///
    /// Returns entries in ascending order (oldest first). If `from_block` doesn't exist,
    /// starts from the next available block after it.
    pub fn get_updates_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, ClientUpdateOutput)>> {
        self.ops
            .get_client_updates_from_blocking(from_block, max_count)
    }
}

/// Internally tracks the current state so we can fetch it as needed.
#[derive(Debug)]
struct CurStateTracker {
    last_idx: Option<L1Height>,
    state: Option<Arc<ClientState>>,
}

impl CurStateTracker {
    fn new_empty() -> Self {
        Self {
            last_idx: None,
            state: None,
        }
    }

    fn set(&mut self, idx: L1Height, state: Arc<ClientState>) {
        self.last_idx = Some(idx);
        self.state = Some(state);
    }

    fn is_idx_better(&self, idx: L1Height) -> bool {
        self.last_idx.is_none_or(|v| idx >= v)
    }

    fn maybe_update(&mut self, idx: L1Height, state: &Arc<ClientState>) -> bool {
        let should = self.is_idx_better(idx);
        if should {
            self.set(idx, state.clone());
        }
        should
    }
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::backend::DatabaseBackend;
    use strata_identifiers::L1BlockId;

    use super::*;

    fn setup_manager() -> ClientStateManager {
        let handle = crate::test_runtime_handle();
        let db = Arc::new(get_test_sled_backend());
        ClientStateManager::new(handle, db.client_state_db()).expect("open client state manager")
    }

    fn block_at(height: L1Height) -> L1BlockCommitment {
        L1BlockCommitment::new(
            height,
            L1BlockId::from(strata_identifiers::Buf32::from([height as u8; 32])),
        )
    }

    fn empty_update() -> ClientUpdateOutput {
        ClientUpdateOutput::new(ClientState::new(None, None), vec![])
    }

    #[test]
    fn del_update_removes_row_and_resets_latest() {
        let manager = setup_manager();
        let low = block_at(10);
        let high = block_at(20);

        manager
            .put_update_blocking(&low, empty_update())
            .expect("put low");
        manager
            .put_update_blocking(&high, empty_update())
            .expect("put high");

        manager.del_update_blocking(&high).expect("delete high");

        assert!(
            manager
                .get_update_blocking(&high)
                .expect("query high")
                .is_none(),
            "deleted row must be gone"
        );
        let (latest_block, _) = manager
            .fetch_most_recent_state()
            .expect("query latest")
            .expect("latest row");
        assert_eq!(
            latest_block, low,
            "latest must fall back to the remaining row"
        );
    }
}
