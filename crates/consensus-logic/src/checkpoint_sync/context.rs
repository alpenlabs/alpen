use std::sync::Arc;

use anyhow::anyhow;
use bitcoin::Transaction;
use bitcoind_async_client::{client::Client, traits::Reader};
use strata_asm_common::{AsmManifest, TxInputRef};
use strata_asm_txs_checkpoint::extract_checkpoint_from_envelope;
use strata_btc_types::{Buf32BitcoinExt, RawBitcoinTx};
use strata_chain_worker_new::ChainWorkerHandle;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::DbResult;
use strata_identifiers::CheckpointL1Ref;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_ol_da::{DAExtractor, DaExtractorError, DaExtractorResult, ExtractedDA, decode_ol_da_payload_bytes};
use strata_ol_state_types::OLState;
use strata_primitives::{EpochCommitment, L1Height, OLBlockCommitment};
use strata_storage::NodeStorage;
use tokio::runtime::Handle;

pub trait CheckpointSyncCtx {
    /// Getter for chain worker handle reference.
    fn chain_worker(&self) -> &ChainWorkerHandle;

    /// Gets the corresponding epoch summary. If not found, returns error.
    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary>;

    /// Extract da given the extractor.
    fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA>;

    /// Gets state at given `OLBlockCommitment`.
    fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState>;

    /// Gets asm manifests for a range.
    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>>;
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
}

impl<E: DAExtractor> CheckpointSyncCtxImpl<E> {
    pub fn new(
        storage: Arc<NodeStorage>,
        chain_worker: Arc<ChainWorkerHandle>,
        da_extractor: E,
    ) -> Self {
        Self {
            storage,
            chain_worker,
            da_extractor,
        }
    }
}

impl<E: DAExtractor> CheckpointSyncCtx for CheckpointSyncCtxImpl<E> {
    fn chain_worker(&self) -> &ChainWorkerHandle {
        &self.chain_worker
    }

    fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<EpochSummary> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(epoch)?
            .ok_or(strata_db_types::DbError::NonExistentEntry)
    }

    fn extract_da_data(&self, ckpt_ref: &CheckpointL1Ref) -> anyhow::Result<ExtractedDA> {
        self.da_extractor
            .extract_da(ckpt_ref)
            .map_err(|e| anyhow!("DA extraction failed: {e}"))
    }

    fn get_state_at(&self, blkid: OLBlockCommitment) -> anyhow::Result<OLState> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(blkid)?
            .ok_or_else(|| anyhow!("missing OL state for {blkid:?}"))?;
        Ok((*state).clone())
    }

    fn fetch_asm_manifests_range(
        &self,
        start: L1Height,
        end: L1Height,
    ) -> anyhow::Result<Vec<AsmManifest>> {
        let l1_mgr = self.storage.l1();
        let mut manifests = Vec::new();
        for height in start..=end {
            let manifest = l1_mgr
                .get_block_manifest_at_height(height)?
                .ok_or_else(|| anyhow!("missing ASM manifest at L1 height {height}"))?;
            manifests.push(manifest);
        }
        Ok(manifests)
    }
}

/// Concrete [`DAExtractor`] that fetches raw Bitcoin transactions via RPC and
/// decodes the OL DA payload + terminal header complement.
#[derive(Clone, Debug)]
pub struct BitcoinDAExtractor {
    client: Arc<Client>,
    magic_bytes: MagicBytes,
    handle: Handle,
}

impl BitcoinDAExtractor {
    pub fn new(client: Arc<Client>, magic_bytes: MagicBytes, handle: Handle) -> Self {
        Self {
            client,
            magic_bytes,
            handle,
        }
    }
}

impl DAExtractor for BitcoinDAExtractor {
    fn extract_da(&self, ckpt_ref: &CheckpointL1Ref) -> DaExtractorResult<ExtractedDA> {
        let txid = ckpt_ref.txid.to_txid();

        let raw_tx_resp = self
            .handle
            .block_on(self.client.get_raw_transaction_verbosity_zero(&txid))
            .map_err(|e| DaExtractorError::Other(format!("failed to fetch tx {txid}: {e}")))?;

        let raw_tx = RawBitcoinTx::from(raw_tx_resp.0);
        let tx: Transaction = raw_tx.try_into()?;

        let tag = ParseConfig::new(self.magic_bytes).try_parse_tx(&tx)?;
        let envelope = extract_checkpoint_from_envelope(&TxInputRef::new(&tx, tag))?;

        let sidecar = envelope.payload.sidecar();
        let da_payload = decode_ol_da_payload_bytes(sidecar.ol_state_diff())?;
        let complement = sidecar.terminal_header_complement().clone();

        Ok(ExtractedDA::new(da_payload, complement))
    }
}
