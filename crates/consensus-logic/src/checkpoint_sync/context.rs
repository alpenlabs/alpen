use std::{future::Future, sync::Arc};

use anyhow::anyhow;
use bitcoin::Transaction;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::{AsmManifest, TxInputRef};
use strata_asm_txs_checkpoint::extract_checkpoint_from_envelope;
use strata_btc_types::{Buf32BitcoinExt, RawBitcoinTx};
use strata_chain_worker_new::{ApplyDAPayload, ChainWorkerHandle};
use strata_checkpoint_types::EpochSummary;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::{CheckpointL1Ref, Epoch};
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_ol_da::{
    decode_ol_da_payload_bytes, DAExtractor, DaExtractorError, DaExtractorResult, ExtractedDA,
};
use strata_ol_state_types::OLState;
use strata_params::RollupParams;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_service::ServiceMonitor;
use strata_status::{OLSyncStatus, OLSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tracing::debug;

pub trait CheckpointSyncCtx: Send + Sync {
    /// Rollup params.
    fn rollup_params(&self) -> &RollupParams;

    /// Fetches L1 tip height.
    fn fetch_l1_tip_height(&self) -> impl Future<Output = anyhow::Result<L1Height>> + Send;

    /// Getter for current csm status.
    fn fetch_csm_status(&self) -> impl Future<Output = anyhow::Result<CsmWorkerStatus>> + Send;

    /// Gets the canonical epoch commitment for given epoch number.
    fn get_canonical_epoch_commitment(
        &self,
        ep: Epoch,
    ) -> impl Future<Output = DbResult<Option<EpochCommitment>>> + Send;

    /// Gets the L1 reference of the epoch commitment if present
    fn get_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = DbResult<Option<CheckpointL1Ref>>> + Send;

    /// Gets the corresponding epoch summary. If not found, returns error.
    fn get_epoch_summary(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = DbResult<Option<EpochSummary>>> + Send;

    /// Extract da given the extractor.
    fn extract_da_data(
        &self,
        ckpt_ref: &CheckpointL1Ref,
    ) -> impl Future<Output = anyhow::Result<ExtractedDA>> + Send;

    /// Gets state at given `OLBlockCommitment`.
    fn get_state_at(
        &self,
        blkid: OLBlockCommitment,
    ) -> impl Future<Output = anyhow::Result<OLState>> + Send;

    /// Gets asm manifests for a range.
    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> impl Future<Output = anyhow::Result<Vec<AsmManifest>>> + Send;

    /// Publishes the OL sync status update to the status channel.
    fn publish_ol_sync_status(&self, status: OLSyncStatus);

    /// Gets L1 reference for given epoch commitment.
    fn fetch_l1_reference(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = anyhow::Result<Option<CheckpointL1Ref>>> + Send;

    /// Submits a DA payload to the chain worker.
    fn apply_da(
        &self,
        payload: &ApplyDAPayload,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Updates the safe tip in the chain worker.
    fn update_safe_tip(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Finalizes an epoch in the chain worker.
    fn finalize_epoch(
        &self,
        epoch: EpochCommitment,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug impls"
)]
pub struct CheckpointSyncCtxImpl<E: DAExtractor> {
    storage: Arc<NodeStorage>,
    chain_worker: Arc<ChainWorkerHandle>,
    da_extractor: E,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    status_channel: Arc<StatusChannel>,
    bitcoin_client: Arc<Client>,
    rollup_params: RollupParams,
}

impl<E: DAExtractor> CheckpointSyncCtxImpl<E> {
    pub fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        da_extractor: E,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        status_channel: Arc<StatusChannel>,
        bitcoin_client: Arc<Client>,
        rollup_params: RollupParams,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            da_extractor,
            csm_monitor,
            status_channel,
            bitcoin_client,
            rollup_params,
        }
    }
}

impl<E: DAExtractor + Send + Sync> CheckpointSyncCtx for CheckpointSyncCtxImpl<E> {
    fn rollup_params(&self) -> &RollupParams {
        &self.rollup_params
    }

    async fn fetch_l1_tip_height(&self) -> anyhow::Result<L1Height> {
        Ok(self.bitcoin_client.get_blockchain_info().await?.blocks)
    }

    async fn fetch_csm_status(&self) -> anyhow::Result<CsmWorkerStatus> {
        Ok(self.csm_monitor.get_current())
    }

    async fn get_canonical_epoch_commitment(&self, ep: Epoch) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(ep)
            .await
    }

    async fn get_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_async(epoch)
            .await
    }

    async fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_async(epoch)
            .await
    }

    async fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA> {
        self.da_extractor
            .extract_da(ckpt_ref)
            .await
            .map_err(|e| anyhow!("DA extraction failed: {e}"))
    }

    async fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_async(blkid)
            .await?
            .ok_or_else(|| anyhow!("missing OL state for {blkid:?}"))?;
        Ok((*state).clone())
    }

    async fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>> {
        let l1_mgr = self.storage.l1();
        let mut manifests = Vec::new();
        for height in start..=end {
            let manifest = l1_mgr
                .get_block_manifest_at_height_async(height)
                .await?
                .ok_or_else(|| anyhow!("missing ASM manifest at L1 height {height}"))?;
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    fn publish_ol_sync_status(&self, status: OLSyncStatus) {
        self.status_channel
            .update_ol_sync_status(OLSyncStatusUpdate::new(status));
    }

    async fn fetch_l1_reference(
        &self,
        epoch: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointL1Ref>> {
        let ckpt_db = self.storage.ol_checkpoint();
        Ok(ckpt_db.get_checkpoint_l1_ref_async(epoch).await?)
    }

    async fn apply_da(&self, payload: &ApplyDAPayload) -> anyhow::Result<()> {
        Ok(self.chain_worker.apply_da(payload).await?)
    }

    async fn update_safe_tip(&self, tip: OLBlockCommitment) -> anyhow::Result<()> {
        Ok(self.chain_worker.update_safe_tip(tip).await?)
    }

    async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
        Ok(self.chain_worker.finalize_epoch(epoch).await?)
    }
}

/// Concrete [`DAExtractor`] that fetches raw Bitcoin transactions via RPC and
/// decodes the OL DA payload + terminal header complement.
#[derive(Clone, Debug)]
pub struct BitcoinDAExtractor {
    client: Arc<Client>,
    magic_bytes: MagicBytes,
}

impl BitcoinDAExtractor {
    pub fn new(client: Arc<Client>, magic_bytes: MagicBytes) -> Self {
        Self {
            client,
            magic_bytes,
        }
    }
}

impl DAExtractor for BitcoinDAExtractor {
    async fn extract_da(&self, ckpt_ref: &CheckpointL1Ref) -> DaExtractorResult<ExtractedDA> {
        let txid = ckpt_ref.txid.to_txid();
        debug!(%txid, "fetching checkpoint tx from Bitcoin");

        let raw_tx_resp = self
            .client
            .get_raw_transaction_verbosity_zero(&txid)
            .await
            .map_err(|e| DaExtractorError::Other(format!("failed to fetch tx {txid}: {e}")))?;

        let raw_tx = RawBitcoinTx::from(raw_tx_resp.0);
        let tx: Transaction = raw_tx.try_into()?;

        let tag = ParseConfig::new(self.magic_bytes).try_parse_tx(&tx)?;
        let envelope = extract_checkpoint_from_envelope(&TxInputRef::new(&tx, tag))?;

        let sidecar = envelope.payload.sidecar();
        let da_payload = decode_ol_da_payload_bytes(sidecar.ol_state_diff())?;
        let complement = sidecar.terminal_header_complement().clone();

        debug!(
            %txid,
            da_size = sidecar.ol_state_diff().len(),
            "extracted DA from checkpoint tx"
        );

        Ok(ExtractedDA::new(da_payload, complement))
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{ScriptBuf, Transaction};
    use strata_asm_txs_checkpoint::{CheckpointTxError, OL_STF_CHECKPOINT_TX_TAG};
    use strata_asm_txs_test_utils::create_reveal_transaction_stub;
    use strata_l1_envelope_fmt::parser::parse_envelope_payload;
    use strata_ol_da::DaExtractorError;

    fn make_checkpoint_tx(payload: &[u8]) -> Transaction {
        create_reveal_transaction_stub(payload.to_vec(), &OL_STF_CHECKPOINT_TX_TAG)
    }

    fn extract_leaf_script(tx: &Transaction) -> Result<ScriptBuf, DaExtractorError> {
        if tx.input.is_empty() {
            return Err(DaExtractorError::CheckpointTxError(
                CheckpointTxError::MissingInputs,
            ));
        }

        tx.input[0]
            .witness
            .taproot_leaf_script()
            .map(|leaf| leaf.script.into())
            .ok_or(DaExtractorError::CheckpointTxError(
                CheckpointTxError::MissingLeafScript,
            ))
    }

    #[test]
    fn test_envelope_roundtrip_large_payload() {
        let payload = vec![0xAB; 1_300];
        assert!(payload.len() > 520, "payload must exceed single push limit");

        let tx = make_checkpoint_tx(&payload);

        let script = extract_leaf_script(&tx).expect("extract envelope-bearing leaf script");
        let parsed_payload = parse_envelope_payload(&script).expect("parse envelope payload");
        assert_eq!(parsed_payload, payload);
    }
}
