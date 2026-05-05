//! Concrete [`CsmWorkerContext`] used by the live node.

use std::sync::Arc;

use bitcoin::Block;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_btc_types::L1BlockIdBitcoinExt;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_csm_worker::CsmWorkerContext;
use strata_params::Params;
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::runtime::Handle;

/// CSM worker context.
#[expect(
    missing_debug_implementations,
    reason = "Inner types don't implement Debug"
)]
pub struct CsmWorkerCtx {
    handle: Handle,
    bitcoin_client: Arc<Client>,
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
}

impl CsmWorkerCtx {
    pub fn new(
        handle: Handle,
        bitcoin_client: Arc<Client>,
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        Self {
            handle,
            bitcoin_client,
            params,
            storage,
            status_channel,
        }
    }
}

impl CsmWorkerContext for CsmWorkerCtx {
    fn put_client_state_update(
        &self,
        block: &L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> anyhow::Result<()> {
        self.storage
            .client_state()
            .put_update_blocking(block, output)?;
        Ok(())
    }

    fn publish_client_state(&self, state: ClientState, block: L1BlockCommitment) {
        self.status_channel.update_client_state(state, block);
    }

    fn put_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
        observation: CheckpointL1Ref,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_ref_blocking(commitment, observation)?;
        Ok(())
    }

    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointPayload>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(commitment)?)
    }

    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_payload_entry_blocking(commitment, payload)?;
        Ok(())
    }

    fn get_l1_block(&self, blockid: &L1BlockId) -> anyhow::Result<Block> {
        let hash = blockid.to_block_hash();
        self.handle
            .block_on(self.bitcoin_client.get_block(&hash))
            .map_err(|e| anyhow::anyhow!("failed to fetch L1 block {blockid}: {e}"))
    }

    fn l1_reorg_safe_depth(&self) -> u32 {
        self.params.rollup.l1_reorg_safe_depth
    }
}
