//! Concrete [`CsmWorkerContext`] used by the live node.

use std::sync::Arc;

use bitcoin::Block;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::AuxData;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_btc_types::L1BlockIdBitcoinExt;
use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff};
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_csm_worker::CsmWorkerContext;
use strata_l1_txfmt::MagicBytes;
use strata_params::Params;
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
};
use strata_state::asm_state::AsmState;
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

    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_observation_blocking(commitment, payload, l1_ref)?;
        Ok(())
    }

    fn get_l1_block(&self, blockid: &L1BlockId) -> anyhow::Result<Block> {
        // ASM has already processed this block, so bitcoind must have it. Retry
        // with backoff to absorb transient failures (RPC timeout, replica lag).
        let hash = blockid.to_block_hash();
        let backoff = ExponentialBackoff::new(200, 15, 10);
        retry_with_backoff("csm_get_l1_block", 10, &backoff, || {
            self.handle
                .block_on(self.bitcoin_client.get_block(&hash))
                .map_err(|e| anyhow::anyhow!("fetch L1 block {blockid}: {e}"))
        })
    }

    fn l1_reorg_safe_depth(&self) -> u32 {
        self.params.rollup.l1_reorg_safe_depth
    }

    fn magic_bytes(&self) -> MagicBytes {
        self.params.rollup.magic_bytes
    }

    fn get_asm_state(&self, block: &L1BlockCommitment) -> anyhow::Result<AsmState> {
        self.storage
            .asm()
            .get_state(*block)?
            .ok_or_else(|| anyhow::anyhow!("missing ASM state for {block}"))
    }

    fn get_aux_data(&self, block: &L1BlockCommitment) -> anyhow::Result<AuxData> {
        self.storage
            .asm()
            .get_aux_data(*block)?
            .ok_or_else(|| anyhow::anyhow!("missing ASM aux data for {block}"))
    }
}
