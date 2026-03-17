//! [`BatchDaProvider`] implementation using chunked envelope inscription.

use std::{collections::HashMap, fmt, sync::Arc};

use alpen_ee_common::{
    prepare_da_chunks, BatchDaProvider, BatchId, DaBlobSource, DaStatus, L1DaBlockRef,
};
use alpen_ee_database::BroadcastDbOps;
use async_trait::async_trait;
use bitcoin::{Txid, Wtxid};
use eyre::{bail, ensure};
use strata_btc_types::Buf32BitcoinExt;
use strata_btcio::writer::chunked_envelope::ChunkedEnvelopeHandle;
use strata_db_types::types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxStatus};
use strata_identifiers::{L1BlockCommitment, L1BlockId, L1Height};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::buf::Buf32;
use tracing::*;

/// Groups reveal txs by L1 block for [`L1DaBlockRef`] construction.
type BlockMap = HashMap<(Buf32, L1Height), Vec<(Txid, Wtxid)>>;

/// [`BatchDaProvider`] that posts DA via chunked envelope inscription.
pub struct ChunkedEnvelopeDaProvider {
    blob_provider: Arc<dyn DaBlobSource>,
    envelope_handle: Arc<ChunkedEnvelopeHandle>,
    broadcast_ops: Arc<BroadcastDbOps>,
    magic_bytes: MagicBytes,
}

impl fmt::Debug for ChunkedEnvelopeDaProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkedEnvelopeDaProvider")
            .field("magic_bytes", &self.magic_bytes)
            .finish_non_exhaustive()
    }
}

impl ChunkedEnvelopeDaProvider {
    pub fn new(
        blob_provider: Arc<dyn DaBlobSource>,
        envelope_handle: Arc<ChunkedEnvelopeHandle>,
        broadcast_ops: Arc<BroadcastDbOps>,
        magic_bytes: MagicBytes,
    ) -> Self {
        Self {
            blob_provider,
            envelope_handle,
            broadcast_ops,
            magic_bytes,
        }
    }
}

fn format_tx_pairs(tx_pairs: &[(Txid, Wtxid)]) -> Vec<String> {
    tx_pairs
        .iter()
        .map(|(txid, wtxid)| format!("{txid}/{wtxid}"))
        .collect()
}

fn format_reveal_refs(entry: &ChunkedEnvelopeEntry) -> Vec<String> {
    entry
        .reveals
        .iter()
        .map(|reveal| format!("{}/{}", reveal.txid, reveal.wtxid))
        .collect()
}

fn format_da_block_refs(block_refs: &[L1DaBlockRef]) -> Vec<String> {
    block_refs
        .iter()
        .map(|block_ref| {
            let reveal_refs = format_tx_pairs(&block_ref.txns);
            format!(
                "{}@{} txns={:?}",
                block_ref.block.height(),
                block_ref.block.blkid(),
                reveal_refs
            )
        })
        .collect()
}

#[async_trait]
impl BatchDaProvider for ChunkedEnvelopeDaProvider {
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<u64> {
        let blob = self.blob_provider.get_blob(batch_id).await?;
        let chunks = prepare_da_chunks(&blob)?;
        ensure!(!chunks.is_empty(), "prepare_da_chunks returned empty");

        let entry = ChunkedEnvelopeEntry::new_unsigned(chunks, self.magic_bytes);
        let chunk_count = entry.chunk_data.len();

        let idx = self
            .envelope_handle
            .submit_entry(entry)
            .await
            .map_err(|e| eyre::eyre!("failed to submit envelope entry: {e}"))?;

        info!(
            component = "alpen_ee_da_provider",
            batch_id = ?batch_id,
            envelope_idx = %idx,
            chunk_count,
            "submitted chunked envelope for batch DA"
        );
        Ok(idx)
    }

    async fn check_da_status(
        &self,
        batch_id: BatchId,
        envelope_idx: u64,
    ) -> eyre::Result<DaStatus> {
        let entry = self
            .envelope_handle
            .ops()
            .get_chunked_envelope_entry_async(envelope_idx)
            .await?;
        let Some(entry) = entry else {
            bail!("envelope entry {envelope_idx} missing from DB for batch {batch_id:?}");
        };

        let reveal_refs = format_reveal_refs(&entry);
        let check_da_status_span = info_span!(
            "alpen_ee_check_da_status",
            component = "alpen_ee_da_provider",
            batch_id = ?batch_id,
            envelope_idx,
            commit_txid = %entry.commit_txid,
            reveal_refs = ?reveal_refs,
        );

        async {
            debug!(status = ?entry.status, "checking chunked envelope status");

            match entry.status {
                ChunkedEnvelopeStatus::Finalized => {
                    let block_refs = self.build_da_block_refs(&entry).await?;
                    let da_block_refs = format_da_block_refs(&block_refs);
                    info!(da_block_refs = ?da_block_refs, "batch DA finalized on L1");
                    Ok(DaStatus::Ready(block_refs))
                }
                ChunkedEnvelopeStatus::Unsigned
                | ChunkedEnvelopeStatus::NeedsResign
                | ChunkedEnvelopeStatus::Unpublished
                | ChunkedEnvelopeStatus::CommitPublished
                | ChunkedEnvelopeStatus::Published
                | ChunkedEnvelopeStatus::Confirmed => Ok(DaStatus::Pending),
            }
        }
        .instrument(check_da_status_span)
        .await
    }
}

impl ChunkedEnvelopeDaProvider {
    /// Builds [`L1DaBlockRef`] entries from broadcast DB for a finalized envelope.
    ///
    /// Collects reveal txs (which carry DA witness data), looks up each in the
    /// broadcast DB to get its L1 block, then groups by block into
    /// [`L1DaBlockRef`] entries. The commit tx is excluded because it only
    /// creates the P2TR output and contains no DA data.
    async fn build_da_block_refs(
        &self,
        entry: &ChunkedEnvelopeEntry,
    ) -> eyre::Result<Vec<L1DaBlockRef>> {
        // Only collect reveal txs — the commit tx is just a P2TR output and
        // carries no DA witness data. The EE prover needs reveal witnesses only.
        let mut tx_pairs: Vec<(Buf32, Buf32)> = Vec::with_capacity(entry.reveals.len());
        for reveal in &entry.reveals {
            tx_pairs.push((reveal.txid, reveal.wtxid));
        }

        // Group by (block_hash, block_height) -> Vec<(Txid, Wtxid)>.
        let mut block_map: BlockMap = HashMap::new();

        for (txid_buf, wtxid_buf) in &tx_pairs {
            let Some(tx_entry) = self
                .broadcast_ops
                .get_tx_entry_by_id_async(*txid_buf)
                .await?
            else {
                bail!("broadcast entry for txid {txid_buf} not found");
            };

            let L1TxStatus::Finalized {
                block_hash,
                block_height,
                ..
            } = tx_entry.status
            else {
                bail!(
                    "expected Finalized status for txid {txid_buf}, got {:?}",
                    tx_entry.status
                );
            };

            block_map
                .entry((block_hash, block_height))
                .or_default()
                .push((txid_buf.to_txid(), wtxid_buf.to_wtxid()));
        }

        // Build sorted L1DaBlockRef list (ascending by block height).
        let mut refs: Vec<L1DaBlockRef> = block_map
            .into_iter()
            .map(|((hash, height), txns)| {
                let commitment = L1BlockCommitment::new(height, L1BlockId::from(hash));
                L1DaBlockRef::new(commitment, txns)
            })
            .collect();
        refs.sort_by_key(|r| r.block.height());

        Ok(refs)
    }
}
