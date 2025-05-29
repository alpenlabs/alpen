use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use bitcoin::{
    consensus::deserialize,
    hashes::Hash,
    key::Parity,
    secp256k1::{PublicKey, XOnlyPublicKey},
    Transaction as BTransaction, Txid,
};
use futures::TryFutureExt;
use jsonrpsee::core::RpcResult;
use strata_btcio::{broadcaster::L1BroadcastHandle, writer::EnvelopeHandle};
#[cfg(feature = "debug-utils")]
use strata_common::BAIL_SENDER;
use strata_common::{send_action_to_worker, Action, WorkerType};
use strata_consensus_logic::{checkpoint_verification::verify_proof, sync_manager::SyncManager};
use strata_db::types::{CheckpointConfStatus, CheckpointProvingStatus, L1TxEntry, L1TxStatus};
use strata_primitives::{
    batch::EpochSummary,
    bridge::{OperatorIdx, PublickeyTable},
    buf::Buf32,
    epoch::EpochCommitment,
    hash,
    l1::payload::{L1Payload, PayloadDest, PayloadIntent},
    params::Params,
};
use strata_rpc_api::{
    StrataAdminApiServer, StrataApiServer, StrataDebugApiServer, StrataSequencerApiServer,
};
use strata_rpc_types::{
    errors::RpcServerError as Error, DaBlob, HexBytes, HexBytes32, HexBytes64, L2BlockStatus,
    RpcBlockHeader, RpcChainState, RpcCheckpointConfStatus, RpcCheckpointInfo, RpcClientStatus,
    RpcDepositEntry, RpcExecUpdate, RpcL1Status, RpcSyncStatus,
};
use strata_rpc_utils::to_jsonrpsee_error;
use strata_sequencer::{
    block_template::{
        BlockCompletionData, BlockGenerationConfig, BlockTemplate, TemplateManagerHandle,
    },
    checkpoint::{verify_checkpoint_sig, CheckpointHandle},
    duty::{extractor::extract_duties, types::Duty},
};
use strata_state::{
    batch::{Checkpoint, SignedCheckpoint},
    block::{L2Block, L2BlockBundle},
    bridge_ops::WithdrawalIntent,
    chain_state::Chainstate,
    client_state::ClientState,
    header::L2Header,
    id::L2BlockId,
    operation::ClientUpdateOutput,
    sync_event::SyncEvent,
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::sync::{oneshot, Mutex};
use tracing::*;
use zkaleido::ProofReceipt;

pub(crate) struct StrataRpcImpl {
    status_channel: StatusChannel,
    sync_manager: Arc<SyncManager>,
    storage: Arc<NodeStorage>,
    checkpoint_handle: Arc<CheckpointHandle>,
    //relayer_handle: Arc<RelayerHandle>,
}

impl StrataRpcImpl {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        status_channel: StatusChannel,
        sync_manager: Arc<SyncManager>,
        storage: Arc<NodeStorage>,
        checkpoint_handle: Arc<CheckpointHandle>,
        //relayer_handle: Arc<RelayerHandle>,
    ) -> Self {
        Self {
            status_channel,
            sync_manager,
            storage,
            checkpoint_handle,
            //relayer_handle,
        }
    }

    /// Gets a ref to the current client state as of the last update.
    async fn get_client_state(&self) -> ClientState {
        self.sync_manager.status_channel().get_cur_client_state()
    }

    // TODO make these not return Arc

    /// Gets a clone of the current client state and fetches the chainstate that
    /// of the L2 block that it considers the tip state.
    // TODO remove this RPC, we aren't supposed to be exposing this
    async fn get_cur_states(&self) -> Result<(ClientState, Option<Arc<Chainstate>>), Error> {
        let cs = self.get_client_state().await;

        if cs.sync().is_none() {
            return Ok((cs, None));
        }

        let chs = self.status_channel.get_cur_tip_chainstate().clone();

        Ok((cs, chs))
    }

    async fn fetch_l2_block_ok(&self, blkid: &L2BlockId) -> Result<L2BlockBundle, Error> {
        self.fetch_l2_block(blkid)
            .await?
            .ok_or(Error::MissingL2Block(*blkid))
    }

    async fn fetch_l2_block(&self, blkid: &L2BlockId) -> Result<Option<L2BlockBundle>, Error> {
        self.storage
            .l2()
            .get_block_data_async(blkid)
            .map_err(Error::Db)
            .await
    }
}

fn conv_blk_header_to_rpc(blk_header: &impl L2Header) -> RpcBlockHeader {
    RpcBlockHeader {
        block_idx: blk_header.slot(),
        timestamp: blk_header.timestamp(),
        block_id: *blk_header.get_blockid().as_ref(),
        prev_block: *blk_header.parent().as_ref(),
        l1_segment_hash: *blk_header.l1_payload_hash().as_ref(),
        exec_segment_hash: *blk_header.exec_payload_hash().as_ref(),
        state_root: *blk_header.state_root().as_ref(),
    }
}

#[async_trait]
impl StrataApiServer for StrataRpcImpl {
    async fn get_blocks_at_idx(&self, idx: u64) -> RpcResult<Vec<HexBytes32>> {
        let l2_blocks = self
            .storage
            .l2()
            .get_blocks_at_height_async(idx)
            .await
            .map_err(Error::Db)?;
        let block_ids = l2_blocks
            .iter()
            .map(HexBytes32::from)
            .collect::<Vec<HexBytes32>>();
        Ok(block_ids)
    }

    async fn protocol_version(&self) -> RpcResult<u64> {
        Ok(1)
    }

    async fn block_time(&self) -> RpcResult<u64> {
        Ok(self.sync_manager.params().rollup.block_time)
    }

    async fn get_l1_status(&self) -> RpcResult<RpcL1Status> {
        let l1s = self.status_channel.get_l1_status();
        Ok(RpcL1Status::from_l1_status(
            l1s,
            self.sync_manager.params().rollup().network,
        ))
    }

    async fn get_l1_connection_status(&self) -> RpcResult<bool> {
        Ok(self.get_l1_status().await?.bitcoin_rpc_connected)
    }

    async fn get_l1_block_hash(&self, height: u64) -> RpcResult<Option<String>> {
        Ok(self
            .storage
            .l1()
            .get_canonical_blockid_at_height_async(height)
            .await
            .map_err(Error::Db)?
            .map(|blockid| blockid.to_string()))
    }

    async fn get_client_status(&self) -> RpcResult<RpcClientStatus> {
        let css = self.status_channel.get_chain_sync_status();
        let cstate = self.status_channel.get_cur_client_state();

        // Define default values for all of the fields that we'll fill in later.
        let mut chain_tip = Buf32::zero();
        let mut chain_tip_slot = 0;
        let mut finalized_blkid = Buf32::zero();
        let mut last_l1_block = Buf32::zero();
        let mut buried_l1_height = 0;
        let mut finalized_epoch = None;
        let mut confirmed_epoch = None;
        let mut tip_l1_block = None;
        let mut buried_l1_block = None;

        // Maybe set the chain tip fields.
        // TODO remove this after actually removing the fields
        if let Some(css) = css {
            chain_tip = (*css.tip_blkid()).into();
            chain_tip_slot = css.tip_slot();
        }

        // Maybe set last L1 block.
        if let Some(block) = cstate.get_tip_l1_block() {
            tip_l1_block = Some(block);
            last_l1_block = (*block.blkid()).into(); // TODO remove
        }

        // Maybe set buried L1 block.
        if let Some(block) = cstate.get_buried_l1_block() {
            buried_l1_block = Some(block);
            buried_l1_height = block.height(); // TODO remove
        }

        // Maybe set confirmed epoch.
        if let Some(last_ckpt) = cstate.get_last_checkpoint() {
            confirmed_epoch = Some(last_ckpt.batch_info.get_epoch_commitment());
        }

        // Maybe set finalized epoch.
        if let Some(fin_ckpt) = cstate.get_apparent_finalized_checkpoint() {
            finalized_epoch = Some(fin_ckpt.batch_info.get_epoch_commitment());
            finalized_blkid = (*fin_ckpt.batch_info.final_l2_block().blkid()).into();
        }

        // FIXME: remove deprecated items
        #[allow(deprecated)]
        Ok(RpcClientStatus {
            chain_tip: chain_tip.into(),
            chain_tip_slot,
            finalized_blkid: *finalized_blkid.as_ref(),
            last_l1_block: last_l1_block.into(),
            finalized_epoch,
            confirmed_epoch,
            buried_l1_height,
            tip_l1_block,
            buried_l1_block,
        })
    }

    async fn get_recent_block_headers(&self, count: u64) -> RpcResult<Vec<RpcBlockHeader>> {
        // FIXME: sync state should have a block number
        let css = self
            .status_channel
            .get_chain_sync_status()
            .ok_or(Error::ClientNotStarted)?;
        let tip_blkid = css.tip_blkid();

        let fetch_limit = self.sync_manager.params().run().l2_blocks_fetch_limit;
        if count > fetch_limit {
            return Err(Error::FetchLimitReached(fetch_limit, count).into());
        }

        let mut output = Vec::new();
        let mut cur_blkid = *tip_blkid;
        while output.len() < count as usize {
            let l2_blk = self.fetch_l2_block_ok(&cur_blkid).await?;
            output.push(conv_blk_header_to_rpc(l2_blk.header()));
            cur_blkid = *l2_blk.header().parent();
            if l2_blk.header().slot() == 0 || Buf32::from(cur_blkid).is_zero() {
                break;
            }
        }

        Ok(output)
    }

    async fn get_headers_at_idx(&self, idx: u64) -> RpcResult<Option<Vec<RpcBlockHeader>>> {
        let css = self
            .status_channel
            .get_chain_sync_status()
            .ok_or(Error::ClientNotStarted)?;
        let tip_blkid = css.tip_blkid();

        // check the tip idx
        let tip_block = self.fetch_l2_block_ok(tip_blkid).await?;
        let tip_idx = tip_block.header().slot();

        if idx > tip_idx {
            return Ok(None);
        }

        let blocks = self
            .storage
            .l2()
            .get_blocks_at_height_async(idx)
            .map_err(Error::Db)
            .await?;

        if blocks.is_empty() {
            return Ok(None);
        }

        let mut headers = Vec::new();
        for blkid in blocks {
            let bundle = self.fetch_l2_block_ok(&blkid).await?;
            headers.push(conv_blk_header_to_rpc(bundle.header()));
        }

        Ok(Some(headers))
    }

    async fn get_header_by_id(&self, blkid: L2BlockId) -> RpcResult<Option<RpcBlockHeader>> {
        let block = self.fetch_l2_block(&blkid).await?;
        Ok(block.map(|block| conv_blk_header_to_rpc(block.header())))
    }

    async fn get_exec_update_by_id(&self, blkid: L2BlockId) -> RpcResult<Option<RpcExecUpdate>> {
        match self.fetch_l2_block(&blkid).await? {
            Some(block) => {
                let exec_update = block.exec_segment().update();

                let withdrawals = exec_update
                    .output()
                    .withdrawals()
                    .iter()
                    .map(|intent| {
                        WithdrawalIntent::new(
                            *intent.amt(),
                            intent.destination().clone(),
                            *intent.withdrawal_txid(),
                        )
                    })
                    .collect();

                let da_blobs = exec_update
                    .output()
                    .da_blobs()
                    .iter()
                    .map(|blob| DaBlob {
                        dest: blob.dest().into(),
                        blob_commitment: *blob.commitment().as_ref(),
                    })
                    .collect();

                Ok(Some(RpcExecUpdate {
                    update_idx: exec_update.input().update_idx(),
                    entries_root: *exec_update.input().entries_root().as_ref(),
                    extra_payload: exec_update.input().extra_payload().to_vec(),
                    new_state: *exec_update.output().new_state().as_ref(),
                    withdrawals,
                    da_blobs,
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_epoch_commitments(&self, epoch: u64) -> RpcResult<Vec<EpochCommitment>> {
        let commitments = self
            .storage
            .checkpoint()
            .get_epoch_commitments_at(epoch)
            .map_err(Error::Db)
            .await?;
        Ok(commitments)
    }

    async fn get_epoch_summary(
        &self,
        epoch: u64,
        slot: u64,
        terminal: L2BlockId,
    ) -> RpcResult<Option<EpochSummary>> {
        let commitment = EpochCommitment::new(epoch, slot, terminal);
        let summary = self
            .storage
            .checkpoint()
            .get_epoch_summary(commitment)
            .map_err(Error::Db)
            .await?;
        Ok(summary)
    }

    async fn get_chainstate_raw(&self, slot: u64) -> RpcResult<Vec<u8>> {
        let chs = self
            .storage
            .chainstate()
            .get_toplevel_chainstate_async(slot)
            .map_err(Error::Db)
            .await?
            .ok_or(Error::MissingChainstate(slot))?
            .to_chainstate();

        let raw_chs = borsh::to_vec(&chs)
            .map_err(|_| Error::Other("failed to serialize chainstate".to_string()))?;
        Ok(raw_chs)
    }

    // TODO rework this, at least to use new OL naming?
    async fn get_cl_block_witness_raw(&self, blkid: L2BlockId) -> RpcResult<Vec<u8>> {
        let l2_blk_bundle = self.fetch_l2_block_ok(&blkid).await?;

        let prev_slot = l2_blk_bundle.block().header().header().slot() - 1;

        let chain_state = self
            .storage
            .chainstate()
            .get_toplevel_chainstate_async(prev_slot)
            .map_err(Error::Db)
            .await?
            .ok_or(Error::MissingChainstate(prev_slot))?
            .to_chainstate();

        let cl_block_witness = (chain_state, l2_blk_bundle.block());
        let raw_cl_block_witness = borsh::to_vec(&cl_block_witness)
            .map_err(|_| Error::Other("Failed to get raw cl block witness".to_string()))?;

        Ok(raw_cl_block_witness)
    }

    async fn get_current_deposits(&self) -> RpcResult<Vec<u32>> {
        let deps = self
            .status_channel
            .cur_tip_deposits_table()
            .ok_or(Error::BeforeGenesis)?;

        Ok(deps.get_all_deposits_idxs_iters_iter().collect())
    }

    async fn get_current_deposit_by_id(&self, deposit_id: u32) -> RpcResult<RpcDepositEntry> {
        let deps = self
            .status_channel
            .cur_tip_deposits_table()
            .ok_or(Error::BeforeGenesis)?;
        Ok(deps
            .get_deposit(deposit_id)
            .ok_or(Error::UnknownIdx(deposit_id))
            .map(RpcDepositEntry::from_deposit_entry)?)
    }

    // FIXME: remove deprecated
    #[allow(deprecated)]
    async fn sync_status(&self) -> RpcResult<RpcSyncStatus> {
        let cssu = self
            .status_channel
            .get_last_sync_status_update()
            .ok_or(Error::BeforeGenesis)?;

        let css = cssu.new_status();

        Ok(RpcSyncStatus {
            tip_height: css.tip_slot(),
            tip_block_id: *css.tip_blkid(),
            cur_epoch: css.cur_epoch(),
            prev_epoch: css.prev_epoch,
            observed_finalized_epoch: *cssu.new_tl_chainstate().finalized_epoch(),
            safe_l1_block: css.safe_l1,
            finalized_block_id: *css.finalized_blkid(),
        })
    }

    async fn get_raw_bundles(&self, start_height: u64, end_height: u64) -> RpcResult<HexBytes> {
        let block_ids = futures::future::join_all(
            (start_height..=end_height)
                .map(|height| self.storage.l2().get_blocks_at_height_async(height)),
        )
        .await;

        let block_ids = block_ids
            .into_iter()
            .filter_map(|f| f.ok())
            .flatten()
            .collect::<Vec<_>>();

        let blocks = futures::future::join_all(
            block_ids
                .iter()
                .map(|blkid| self.storage.l2().get_block_data_async(blkid)),
        )
        .await;

        let blocks = blocks
            .into_iter()
            .filter_map(|blk| blk.ok().flatten())
            .collect::<Vec<_>>();

        borsh::to_vec(&blocks)
            .map(HexBytes)
            .map_err(to_jsonrpsee_error("failed to serialize"))
    }

    async fn get_raw_bundle_by_id(&self, block_id: L2BlockId) -> RpcResult<Option<HexBytes>> {
        let block = self
            .storage
            .l2()
            .get_block_data_async(&block_id)
            .await
            .map_err(Error::Db)?
            .map(|block| {
                borsh::to_vec(&block)
                    .map(HexBytes)
                    .map_err(to_jsonrpsee_error("failed to serialize"))
            })
            .transpose()?;
        Ok(block)
    }

    async fn get_msgs_by_scope(&self, _scope: HexBytes) -> RpcResult<Vec<HexBytes>> {
        warn!("call to get_msgs_by_scope, bridge relay system deprecated");
        Ok(Vec::new())
    }

    async fn submit_bridge_msg(&self, _raw_msg: HexBytes) -> RpcResult<()> {
        warn!("call to submit_bridge_msg, bridge relay system deprecated");
        Ok(())
    }

    async fn get_active_operator_chain_pubkey_set(&self) -> RpcResult<PublickeyTable> {
        let operator_table = self
            .status_channel
            .cur_tip_operator_table()
            .ok_or(Error::BeforeGenesis)?;
        let operator_map: BTreeMap<OperatorIdx, PublicKey> = operator_table
            .operators()
            .iter()
            .fold(BTreeMap::new(), |mut map, entry| {
                let pubkey = XOnlyPublicKey::try_from(*entry.wallet_pk())
                    .expect("something has gone horribly wrong");

                // This is a taproot pubkey so its parity has to be even.
                let pubkey = pubkey.public_key(Parity::Even);

                map.insert(entry.idx(), pubkey);
                map
            });

        Ok(operator_map.into())
    }

    async fn get_checkpoint_info(&self, idx: u64) -> RpcResult<Option<RpcCheckpointInfo>> {
        let entry = self
            .checkpoint_handle
            .get_checkpoint(idx)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        Ok(entry.map(Into::into))
    }

    async fn get_checkpoint_conf_status(
        &self,
        idx: u64,
    ) -> RpcResult<Option<RpcCheckpointConfStatus>> {
        self.checkpoint_handle
            .get_checkpoint(idx)
            .await
            .map(|opt| opt.map(Into::into))
            .map_err(|e| Error::Checkpoint(e.to_string()).into())
    }

    async fn get_latest_checkpoint_index(&self, finalized: Option<bool>) -> RpcResult<Option<u64>> {
        let finalized = finalized.unwrap_or(false);
        if finalized {
            // FIXME when this was written, by "finalized" they really meant
            // just the confirmed or "last" checkpoint, we'll replicate this
            // behavior for now

            // get last finalized checkpoint index from state
            let (client_state, _) = self.get_cur_states().await?;
            Ok(client_state
                .get_last_checkpoint()
                .map(|checkpoint| checkpoint.batch_info.epoch()))
        } else {
            // get latest checkpoint index from d
            let idx = self
                .checkpoint_handle
                .get_last_checkpoint_idx()
                .await
                .map_err(|e| Error::Other(e.to_string()))?;

            Ok(idx)
        }
    }

    // TODO this logic should be moved into `SyncManager` or *something* that
    // has easier access to the context about block status instead of
    // implementing protocol-aware deliberation in the RPC method impl
    async fn get_l2_block_status(&self, block_slot: u64) -> RpcResult<L2BlockStatus> {
        let css = self
            .status_channel
            .get_chain_sync_status()
            .ok_or(Error::BeforeGenesis)?;
        let cstate = self.status_channel.get_cur_client_state();

        // FIXME when this was written, "finalized" just meant included in a
        // checkpoint, not that the checkpoint was buried, so we're replicating
        // that behavior here
        if let Some(last_checkpoint) = cstate.get_last_checkpoint() {
            if last_checkpoint.batch_info.includes_l2_block(block_slot) {
                return Ok(L2BlockStatus::Finalized(
                    last_checkpoint.l1_reference.block_height(),
                ));
            }
        }

        if let Some(l1_height) = cstate.get_verified_l1_height(block_slot) {
            return Ok(L2BlockStatus::Verified(l1_height));
        }

        if block_slot < css.tip_slot() {
            return Ok(L2BlockStatus::Confirmed);
        }

        Ok(L2BlockStatus::Unknown)
    }

    // FIXME: possibly create a separate rpc type corresponding to SyncEvent
    async fn get_sync_event(&self, idx: u64) -> RpcResult<Option<SyncEvent>> {
        let ev: Option<SyncEvent> = self
            .storage
            .sync_event()
            .get_sync_event_async(idx)
            .await
            .map_err(Error::Db)?;

        Ok(ev)
    }

    async fn get_last_sync_event_idx(&self) -> RpcResult<u64> {
        let last = self
            .storage
            .sync_event()
            .get_last_idx_async()
            .await
            .map_err(Error::Db)?;

        // FIXME returning MAX if we haven't produced one yet, should figure
        // something else out
        Ok(last.unwrap_or(u64::MAX))
    }

    // FIXME: possibly create a separate rpc type corresponding to ClientUpdateOutput
    async fn get_client_update_output(&self, idx: u64) -> RpcResult<Option<ClientUpdateOutput>> {
        Ok(self
            .storage
            .client_state()
            .get_update_async(idx)
            .map_err(Error::Db)
            .await?)
    }
}

pub(crate) struct AdminServerImpl {
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl AdminServerImpl {
    pub(crate) fn new(stop_tx: oneshot::Sender<()>) -> Self {
        Self {
            stop_tx: Mutex::new(Some(stop_tx)),
        }
    }
}

#[async_trait]
impl StrataAdminApiServer for AdminServerImpl {
    async fn stop(&self) -> RpcResult<()> {
        let mut opt = self.stop_tx.lock().await;
        if let Some(stop_tx) = opt.take() {
            if stop_tx.send(()).is_err() {
                warn!("tried to send stop signal, channel closed");
            }
        }
        Ok(())
    }
}

pub(crate) struct SequencerServerImpl {
    envelope_handle: Arc<EnvelopeHandle>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    checkpoint_handle: Arc<CheckpointHandle>,
    template_manager_handle: TemplateManagerHandle,
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    status: StatusChannel,
}

impl SequencerServerImpl {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        envelope_handle: Arc<EnvelopeHandle>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        params: Arc<Params>,
        checkpoint_handle: Arc<CheckpointHandle>,
        template_manager_handle: TemplateManagerHandle,
        storage: Arc<NodeStorage>,
        status: StatusChannel,
    ) -> Self {
        Self {
            envelope_handle,
            broadcast_handle,
            params,
            checkpoint_handle,
            template_manager_handle,
            storage,
            status,
        }
    }
}

#[async_trait]
impl StrataSequencerApiServer for SequencerServerImpl {
    async fn get_last_tx_entry(&self) -> RpcResult<Option<L1TxEntry>> {
        let broadcast_handle: Arc<L1BroadcastHandle> = self.broadcast_handle.clone();
        let txentry = broadcast_handle.get_last_tx_entry().await;
        Ok(txentry.map_err(|e| Error::Other(e.to_string()))?)
    }

    async fn get_tx_entry_by_idx(&self, idx: u64) -> RpcResult<Option<L1TxEntry>> {
        let broadcast_handle = &self.broadcast_handle;
        let txentry = broadcast_handle.get_tx_entry_by_idx_async(idx).await;
        Ok(txentry.map_err(|e| Error::Other(e.to_string()))?)
    }

    async fn submit_da_blob(&self, blob: HexBytes) -> RpcResult<()> {
        let commitment = hash::raw(&blob.0);
        let payload = L1Payload::new_da(blob.0);
        let blobintent = PayloadIntent::new(PayloadDest::L1, commitment, payload);
        // NOTE: It would be nice to return reveal txid from the submit method. But creation of txs
        // is deferred to signer in the writer module
        if let Err(e) = self.envelope_handle.submit_intent_async(blobintent).await {
            return Err(Error::Other(e.to_string()).into());
        }
        Ok(())
    }

    async fn broadcast_raw_tx(&self, rawtx: HexBytes) -> RpcResult<Txid> {
        let tx: BTransaction = deserialize(&rawtx.0).map_err(|e| Error::Other(e.to_string()))?;
        let txid = tx.compute_txid();
        let dbid = *txid.as_raw_hash().as_byte_array();

        let entry = L1TxEntry::from_tx(&tx);

        self.broadcast_handle
            .put_tx_entry(dbid.into(), entry)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        Ok(txid)
    }

    async fn submit_checkpoint_proof(
        &self,
        ckpt: u64,
        proof_receipt: ProofReceipt,
    ) -> RpcResult<()> {
        // TODO shift all this logic somewhere else that's closer to where it's
        // relevant and not in the RPC method impls

        let span = debug_span!("accept-ckpt-proof", %ckpt);

        let fut = async {
            debug!(%ckpt, "received checkpoint proof request");

            let mut entry = self
                .checkpoint_handle
                .get_checkpoint(ckpt)
                .await
                .map_err(|e| Error::Other(e.to_string()))?
                .ok_or(Error::MissingCheckpointInDb(ckpt))?;
            trace!("found checkpoint in db");

            // If proof is not pending error out.
            if entry.proving_status != CheckpointProvingStatus::PendingProof {
                warn!("already have proof?");
                return Err(Error::ProofAlreadyCreated(ckpt))?;
            }

            let checkpoint = entry.clone().into_batch_checkpoint();
            verify_proof(&checkpoint, &proof_receipt, self.params.rollup()).map_err(|e| {
                warn!("proof is invalid");
                Error::InvalidProof(ckpt, e.to_string())
            })?;

            entry.checkpoint.set_proof(proof_receipt.proof().clone());
            entry.proving_status = CheckpointProvingStatus::ProofReady;

            trace!("proof is pending, setting proof ready");

            self.checkpoint_handle
                .put_checkpoint(ckpt, entry)
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
            debug!("proof stored successfully");

            Ok(())
        }
        .instrument(span);

        fut.await
    }

    async fn get_tx_status(&self, txid: HexBytes32) -> RpcResult<Option<L1TxStatus>> {
        let mut txid = txid.0;
        txid.reverse();
        let id = Buf32::from(txid);
        Ok(self
            .broadcast_handle
            .get_tx_status(id)
            .await
            .map_err(|e| Error::Other(e.to_string()))?)
    }

    async fn get_sequencer_duties(&self) -> RpcResult<Vec<Duty>> {
        let (chain_state, tip_blockid) = self
            .status
            .get_cur_tip_chainstate_with_block()
            .ok_or(Error::BeforeGenesis)?;
        let client_state = self.status.get_cur_client_state();

        let client_int_state = client_state
            .get_last_internal_state()
            .ok_or(Error::MissingInternalState)?;

        let duties = extract_duties(
            chain_state.chain_tip_slot(),
            tip_blockid,
            client_int_state,
            &self.checkpoint_handle,
            self.storage.l2().as_ref(),
            &self.params,
        )
        .await
        .map_err(to_jsonrpsee_error("failed to extract duties"))?;

        Ok(duties)
    }

    async fn get_block_template(&self, config: BlockGenerationConfig) -> RpcResult<BlockTemplate> {
        self.template_manager_handle
            .generate_block_template(config)
            .await
            .map_err(to_jsonrpsee_error(""))
    }

    async fn complete_block_template(
        &self,
        template_id: L2BlockId,
        completion: BlockCompletionData,
    ) -> RpcResult<L2BlockId> {
        self.template_manager_handle
            .complete_block_template(template_id, completion)
            .await
            .map_err(to_jsonrpsee_error("failed to complete block template"))
    }

    async fn complete_checkpoint_signature(
        &self,
        checkpoint_idx: u64,
        sig: HexBytes64,
    ) -> RpcResult<()> {
        // TODO shift all this logic somewhere else that's closer to where it's
        // relevant and not in the RPC method impls
        trace!(%checkpoint_idx, ?sig, "call to complete_checkpoint_signature");

        let entry = self
            .checkpoint_handle
            .get_checkpoint(checkpoint_idx)
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .ok_or(Error::MissingCheckpointInDb(checkpoint_idx))?;

        if entry.proving_status != CheckpointProvingStatus::ProofReady {
            Err(Error::MissingCheckpointProof(checkpoint_idx))?;
        }

        if entry.confirmation_status != CheckpointConfStatus::Pending {
            Err(Error::CheckpointAlreadyPosted(checkpoint_idx))?;
        }

        let checkpoint = Checkpoint::from(entry);
        let signed_checkpoint = SignedCheckpoint::new(checkpoint, sig.0.into());

        if !verify_checkpoint_sig(&signed_checkpoint, &self.params) {
            Err(Error::InvalidCheckpointSignature(checkpoint_idx))?;
        }

        trace!(%checkpoint_idx, "signature OK");

        let payload = L1Payload::new_checkpoint(
            borsh::to_vec(&signed_checkpoint).map_err(|e| Error::Other(e.to_string()))?,
        );
        let sighash = signed_checkpoint.checkpoint().hash();

        let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, payload);
        self.envelope_handle
            .submit_intent_async(payload_intent)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        Ok(())
    }
}

pub(crate) struct StrataDebugRpcImpl {
    storage: Arc<NodeStorage>,
}

impl StrataDebugRpcImpl {
    pub(crate) const fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl StrataDebugApiServer for StrataDebugRpcImpl {
    async fn get_block_by_id(&self, block_id: L2BlockId) -> RpcResult<Option<L2Block>> {
        let l2_block = self
            .storage
            .l2()
            .get_block_data_async(&block_id)
            .await
            .map_err(Error::Db)?
            .map(|b| b.block().clone());
        Ok(l2_block)
    }

    async fn get_chainstate_at_idx(&self, idx: u64) -> RpcResult<Option<RpcChainState>> {
        let chain_state_res = self
            .storage
            .chainstate()
            .get_toplevel_chainstate_async(idx)
            .map_err(Error::Db)
            .await?;
        match chain_state_res {
            Some(cs_entry) => Ok(Some(RpcChainState {
                tip_blkid: *cs_entry.tip_blockid(),
                tip_slot: cs_entry.state().chain_tip_slot(),
                cur_epoch: cs_entry.state().cur_epoch(),
            })),
            None => Ok(None),
        }
    }

    async fn get_clientstate_at_idx(&self, idx: u64) -> RpcResult<Option<ClientState>> {
        Ok(self
            .storage
            .client_state()
            .get_state_async(idx)
            .map_err(Error::Db)
            .await?)
    }

    async fn set_bail_context(&self, _ctx: String) -> RpcResult<()> {
        #[cfg(feature = "debug-utils")]
        let _sender = BAIL_SENDER.send(Some(_ctx));
        Ok(())
    }

    async fn pause_resume_worker(&self, wtype: WorkerType, action: Action) -> RpcResult<bool> {
        Ok(send_action_to_worker(wtype, action).await)
    }
}
