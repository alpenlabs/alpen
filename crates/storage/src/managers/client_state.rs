//! Client state manager.

use std::sync::Arc;

use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db_types::client_state::ClientStateDatabase;
use strata_db_types::DbResult;
use strata_primitives::l1::L1BlockCommitment;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

use crate::cache;
use crate::ops::client_state::ClientStateOps;

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ClientStateManager {
    ops: ClientStateOps,

    update_cache: cache::CacheTable<L1BlockCommitment, Option<ClientUpdateOutput>>,
    cur_state: Mutex<CurStateTracker>,
}

impl ClientStateManager {
    pub fn new(handle: Handle, db: Arc<impl ClientStateDatabase + 'static>) -> DbResult<Self> {
        let ops = ClientStateOps::new(handle, db);
        let update_cache = cache::CacheTable::new(64.try_into().unwrap());

        // Setup the tracker to point at the last or default pregenesis client state.
        let mut cur_state = CurStateTracker::new_empty();

        let latest_cs = ops.get_latest_client_state_blocking()?;
        if let Some((blk, cs)) = latest_cs {
            cur_state.set(blk, Arc::new(cs));
        }

        Ok(Self {
            ops,
            update_cache,
            cur_state: Mutex::new(cur_state),
        })
    }

    pub async fn get_state_async(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .get_update_async(&block)
            .await?
            .map(|update| update.into_state()))
    }

    pub fn get_state_blocking(&self, block: L1BlockCommitment) -> DbResult<Option<ClientState>> {
        Ok(self
            .get_update_blocking(&block)?
            .map(|update| update.into_state()))
    }

    pub async fn get_update_async(
        &self,
        block: &L1BlockCommitment,
    ) -> DbResult<Option<ClientUpdateOutput>> {
        self.update_cache
            .get_or_fetch(block, || self.ops.get_client_update_async(*block))
            .await
    }

    pub fn get_update_blocking(
        &self,
        block: &L1BlockCommitment,
    ) -> DbResult<Option<ClientUpdateOutput>> {
        self.update_cache
            .get_or_fetch_blocking(block, || self.ops.get_client_update_blocking(*block))
    }

    pub fn put_update_blocking(
        &self,
        block: &L1BlockCommitment,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        let state = Arc::new(update.state().clone());
        self.ops
            .put_client_update_blocking(*block, update.clone())?;
        self.maybe_update_cur_state_blocking(block, &state);
        self.update_cache.insert_blocking(*block, Some(update));
        Ok(state)
    }

    pub async fn put_update_async(
        &self,
        block: &L1BlockCommitment,
        update: ClientUpdateOutput,
    ) -> DbResult<Arc<ClientState>> {
        let state = Arc::new(update.state().clone());
        self.ops
            .put_client_update_async(*block, update.clone())
            .await?;
        self.maybe_update_cur_state_async(block, &state).await;
        self.update_cache.insert_async(*block, Some(update)).await;
        Ok(state)
    }

    /// Deletes the client update at `block`, keeping caches and the current
    /// state tracker coherent.
    pub fn del_update_blocking(&self, block: &L1BlockCommitment) -> DbResult<()> {
        self.ops.del_client_update_blocking(*block)?;
        self.update_cache.purge_blocking(block);
        self.reset_cur_state_blocking()?;
        Ok(())
    }

    fn maybe_update_cur_state_blocking(
        &self,
        block: &L1BlockCommitment,
        state: &Arc<ClientState>,
    ) -> bool {
        let mut cur = self.cur_state.blocking_lock();
        cur.maybe_update(*block, state)
    }

    async fn maybe_update_cur_state_async(
        &self,
        block: &L1BlockCommitment,
        state: &Arc<ClientState>,
    ) -> bool {
        let mut cur = self.cur_state.lock().await;
        cur.maybe_update(*block, state)
    }

    /// Recomputes the current-state tracker from storage after a deletion.
    fn reset_cur_state_blocking(&self) -> DbResult<()> {
        let mut cur = self.cur_state.blocking_lock();
        *cur = CurStateTracker::new_empty();
        if let Some((blk, cs)) = self.ops.get_latest_client_state_blocking()? {
            cur.set(blk, Arc::new(cs));
        }
        Ok(())
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the greatest key.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        let latest = self.cur_state.blocking_lock();
        if let Some((block, state)) = latest.as_parts() {
            return Ok(Some((block, state.clone())));
        }

        self.ops.get_latest_client_state_blocking()
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the greatest key.
    pub async fn fetch_most_recent_state_async(
        &self,
    ) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        let latest = self.cur_state.lock().await;
        if let Some((block, state)) = latest.as_parts() {
            return Ok(Some((block, state.clone())));
        }

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
    state: Option<CurState>,
}

#[derive(Debug)]
struct CurState {
    last_block: L1BlockCommitment,
    state: Arc<ClientState>,
}

impl CurStateTracker {
    fn new_empty() -> Self {
        Self { state: None }
    }

    fn set(&mut self, block: L1BlockCommitment, state: Arc<ClientState>) {
        self.state = Some(CurState {
            last_block: block,
            state,
        })
    }

    /// Whether `block` should become the tracked latest.
    ///
    /// Compares the full [`L1BlockCommitment`] (height and block id),
    /// so the tracker mirrors the DB's greatest-key row and stays consistent
    /// to what `get_latest_client_state` would return.
    fn is_block_better(&self, block: L1BlockCommitment) -> bool {
        self.state.as_ref().is_none_or(|s| block >= s.last_block)
    }

    fn maybe_update(&mut self, block: L1BlockCommitment, state: &Arc<ClientState>) -> bool {
        let should = self.is_block_better(block);
        if should {
            self.set(block, state.clone());
        }
        should
    }

    fn as_parts(&self) -> Option<(L1BlockCommitment, &ClientState)> {
        self.state
            .as_ref()
            .map(|s| (s.last_block, s.state.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::backend::DatabaseBackend;
    use strata_identifiers::L1BlockId;
    use strata_primitives::L1Height;

    use super::*;

    fn setup_manager() -> ClientStateManager {
        let handle = crate::test_runtime_handle();
        let db = Arc::new(get_test_sled_backend());
        ClientStateManager::new(handle, db.client_state_db()).expect("open client state manager")
    }

    fn block_at(height: L1Height) -> L1BlockCommitment {
        block_at_id(height, height as u8)
    }

    fn block_at_id(height: L1Height, id_byte: u8) -> L1BlockCommitment {
        L1BlockCommitment::new(
            height,
            L1BlockId::from(strata_identifiers::Buf32::from([id_byte; 32])),
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

    // The latest tracker must key on the full block commitment, not just height.
    // A lower-keyed sibling written after a higher-keyed one at the same height
    // must not overwrite the tracker, or the fast path would diverge from the
    // DB's greatest-key row (`get_latest_client_state`).
    #[test]
    fn latest_tracks_greatest_full_key_at_same_height() {
        let manager = setup_manager();
        let high = block_at_id(10, 0xff);
        let low = block_at_id(10, 0x01);
        assert!(high > low, "sanity: same height, high blkid sorts greater");

        // Write the higher-keyed sibling first, then the lower-keyed one.
        manager
            .put_update_blocking(&high, empty_update())
            .expect("put high");
        manager
            .put_update_blocking(&low, empty_update())
            .expect("put low");

        let (latest_block, _) = manager
            .fetch_most_recent_state()
            .expect("query latest")
            .expect("latest row");
        assert_eq!(
            latest_block, high,
            "latest must be the greatest full key, not the last-written sibling"
        );
    }
}
