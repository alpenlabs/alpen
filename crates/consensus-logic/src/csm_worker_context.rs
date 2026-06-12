//! Concrete [`CsmWorkerContext`] used by the live node.

use std::sync::Arc;

use bitcoin::Block;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::AuxData;
use strata_asm_params::AsmParams;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_btc_types::L1BlockIdBitcoinExt;
use strata_common::retry::{policies::ExponentialBackoff, retry_with_backoff};
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_csm_worker::{CsmWorkerContext, CsmWorkerError, CsmWorkerResult};
use strata_identifiers::Epoch;
use strata_l1_txfmt::MagicBytes;
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
    L1Height,
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
pub struct CsmWorkerContextImpl {
    handle: Handle,
    bitcoin_client: Arc<Client>,
    asm_params: Arc<AsmParams>,
    l1_reorg_safe_depth: u32,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
}

impl CsmWorkerContextImpl {
    pub fn new(
        handle: Handle,
        bitcoin_client: Arc<Client>,
        asm_params: Arc<AsmParams>,
        l1_reorg_safe_depth: u32,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> Self {
        Self {
            handle,
            bitcoin_client,
            asm_params,
            l1_reorg_safe_depth,
            storage,
            status_channel,
        }
    }
}

impl CsmWorkerContext for CsmWorkerContextImpl {
    fn put_client_state_update(
        &self,
        block: &L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> CsmWorkerResult<()> {
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
    ) -> CsmWorkerResult<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_observation_blocking(commitment, payload, l1_ref)?;
        Ok(())
    }

    fn get_l1_block(&self, blockid: &L1BlockId) -> CsmWorkerResult<Block> {
        // ASM has already processed this block, so bitcoind must have it. Retry
        // with backoff to absorb transient failures (RPC timeout, replica lag).
        let hash = blockid.to_block_hash();
        let backoff = ExponentialBackoff::new(200, 15, 10);
        retry_with_backoff("csm_get_l1_block", 10, &backoff, || {
            self.handle
                .block_on(self.bitcoin_client.get_block(&hash))
                .map_err(|e| CsmWorkerError::L1Fetch {
                    blockid: *blockid,
                    cause: e.to_string(),
                })
        })
    }

    fn l1_reorg_safe_depth(&self) -> u32 {
        self.l1_reorg_safe_depth
    }

    fn magic_bytes(&self) -> MagicBytes {
        self.asm_params.magic
    }

    fn get_asm_state(&self, block: &L1BlockCommitment) -> CsmWorkerResult<AsmState> {
        self.storage
            .asm()
            .get_state_blocking(*block)?
            .ok_or_else(|| CsmWorkerError::MissingData {
                what: "ASM state",
                detail: block.to_string(),
            })
    }

    fn get_aux_data(&self, block: &L1BlockCommitment) -> CsmWorkerResult<AuxData> {
        self.storage
            .asm()
            .get_aux_data_blocking(*block)?
            .ok_or_else(|| CsmWorkerError::MissingData {
                what: "ASM aux data",
                detail: block.to_string(),
            })
    }

    fn get_canonical_l1_block(&self, height: L1Height) -> CsmWorkerResult<L1BlockCommitment> {
        let blkid = self
            .storage
            .l1()
            .get_canonical_blockid_at_height(height)?
            .ok_or_else(|| CsmWorkerError::MissingData {
                what: "canonical L1 block",
                detail: format!("height {height}"),
            })?;
        Ok(L1BlockCommitment::new(height, blkid))
    }

    fn fetch_most_recent_client_state(
        &self,
    ) -> CsmWorkerResult<Option<(L1BlockCommitment, ClientState)>> {
        Ok(self.storage.client_state().fetch_most_recent_state()?)
    }

    fn get_client_state_at(
        &self,
        block: &L1BlockCommitment,
    ) -> CsmWorkerResult<Option<ClientState>> {
        Ok(self.storage.client_state().get_state_blocking(*block)?)
    }

    fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.asm_params.anchor.block
    }

    fn get_last_checkpoint_l1_ref_epoch(&self) -> CsmWorkerResult<Option<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_last_checkpoint_l1_ref_epoch_blocking()?)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> CsmWorkerResult<Option<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(epoch)?)
    }

    fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> CsmWorkerResult<Option<CheckpointL1Ref>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)?)
    }

    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> CsmWorkerResult<Option<CheckpointPayload>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_l1_observed_payload_blocking(commitment)?)
    }
}
