//! Client state manager.

use std::sync::Arc;

use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db_types::{traits::ClientStateDatabase, DbResult};
use strata_primitives::l1::L1BlockCommitment;
use threadpool::ThreadPool;

use crate::ops::client_state::{ClientStateOps, Context};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ClientStateManager {
    ops: ClientStateOps,
}

impl ClientStateManager {
    pub fn new(pool: ThreadPool, db: Arc<impl ClientStateDatabase + 'static>) -> DbResult<Self> {
        let ops = Context::new(db).into_ops(pool);

        Ok(Self { ops })
    }

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
    ) -> DbResult<()> {
        self.ops.put_client_update_blocking(*block, update)?;
        Ok(())
    }

    /// Returns either pre-genesis init [`ClientState`] or the one with the biggest height.
    pub fn fetch_most_recent_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        self.ops.get_latest_client_state_blocking()
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
