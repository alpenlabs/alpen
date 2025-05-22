use std::sync::Arc;

use strata_db::DbError;
use strata_primitives::params::Params;
use strata_state::{
    chain_state::Chainstate,
    header::L2Header,
    state_op::{WriteBatch, WriteBatchEntry},
    traits::ChainstateUpdate,
};
use strata_storage::NodeStorage;

use crate::{errors::Error, genesis::make_l2_genesis};

// should we maintain an in-memory structure for this?
pub struct CheckpointSyncManager {
    /// Common node storage interface.
    storage: Arc<NodeStorage>,
}

impl CheckpointSyncManager {
    pub fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    /// Apply chainstate update to latest chainstate.
    pub async fn apply_chainstate_update(
        &mut self,
        chainstate_update: impl ChainstateUpdate,
    ) -> anyhow::Result<Chainstate> {
        let latest_chainstate = self.get_latest_chainstate().await?;
        let new_chainstate = chainstate_update.apply_to_chainstate(latest_chainstate.as_ref());
        Ok(new_chainstate)
    }

    /// Store chainstate to database.
    pub async fn store_chainstate(&mut self, new_chainstate: Chainstate) -> anyhow::Result<()> {
        let chsman = self.storage.chainstate();
        let block_commitment = new_chainstate.finalized_epoch().to_block_commitment();
        let wb = WriteBatchEntry::new(
            WriteBatch::new(new_chainstate.clone(), Vec::new()),
            block_commitment.blkid().to_owned(),
        );
        chsman
            .put_write_batch_async(new_chainstate.chain_tip_slot(), wb)
            .await?;
        Ok(())
    }

    /// Get latest stored chainstate.
    async fn get_latest_chainstate(&self) -> anyhow::Result<Option<Chainstate>> {
        let chsman = self.storage.chainstate();
        if let Ok(idx) = chsman.get_last_write_idx_async().await {
            let latest_chainstate = chsman
                .get_toplevel_chainstate_async(idx)
                .await?
                .ok_or(DbError::MissingL2State(idx))?
                .to_chainstate();
            return Ok(Some(latest_chainstate));
        }

        Ok(None)
    }
}
