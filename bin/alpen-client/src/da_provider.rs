//! DA provider for the alpen EE client.
//!
//! Contains [`ChunkedEnvelopeDaProvider`], the [`BatchDaProvider`] implementation that
//! splits DA blobs into chunks using [`prepare_da_chunks`] and submits them as
//! [`ChunkedEnvelopeEntry`] records. The btcio watcher task picks up unsigned
//! entries, builds commit+reveal txs, and drives them through the broadcast
//! lifecycle.
//!
//! Also contains [`StateDiffBlobProvider`], the concrete [`DaBlobProvider`]
//! implementation that builds encoded DA blobs from per-block state diffs.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use alloy_primitives::B256;
use alpen_ee_common::{
    prepare_da_chunks, BatchDaProvider, BatchId, BatchStorage, DaBlobProvider, DaStatus,
    L1DaBlockRef,
};
use alpen_reth_db::StateDiffProvider;
use alpen_reth_statediff::{BatchBuilder, BatchStateDiff};
use async_trait::async_trait;
use bitcoin::{Txid, Wtxid};
use eyre::{bail, ensure};
use strata_btcio::writer::chunked_envelope::ChunkedEnvelopeHandle;
use strata_codec::encode_to_vec;
use strata_db_types::types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxStatus};
use strata_identifiers::{L1BlockCommitment, L1BlockId};
use strata_primitives::buf::Buf32;
use alpen_ee_database::BroadcastDbOps;
use tokio::{runtime::Handle, task::block_in_place};
use tracing::*;

/// [`DaBlobProvider`] that builds encoded DA blobs from Reth state diffs.
///
/// For each batch, it:
/// 1. Retrieves the block range from [`BatchStorage`].
/// 2. Fetches per-block [`BlockStateChanges`] from the [`StateDiffProvider`].
/// 3. Aggregates them into a [`BatchStateDiff`] via [`BatchBuilder`].
/// 4. Returns the `strata-codec` encoded bytes.
pub(crate) struct StateDiffBlobProvider<S, D> {
    batch_storage: Arc<S>,
    state_diff_provider: Arc<D>,
}

impl<S, D> StateDiffBlobProvider<S, D> {
    pub(crate) fn new(batch_storage: Arc<S>, state_diff_provider: Arc<D>) -> Self {
        Self {
            batch_storage,
            state_diff_provider,
        }
    }
}

impl<S, D> DaBlobProvider for StateDiffBlobProvider<S, D>
where
    S: BatchStorage,
    D: StateDiffProvider + Send + Sync,
{
    fn get_blob(&self, batch_id: BatchId) -> eyre::Result<Option<Vec<u8>>> {
        // 1. Look up the batch to get its block range. `BatchStorage` is async but
        //    `DaBlobProvider::get_blob` is sync, so we bridge via `block_in_place` + `block_on` to
        //    avoid deadlocking the Tokio runtime when all worker threads are occupied.
        let (batch, _status) = block_in_place(|| {
            Handle::current().block_on(self.batch_storage.get_batch_by_id(batch_id))
        })?
        .ok_or_else(|| eyre::eyre!("batch {batch_id:?} not found in storage"))?;

        // 2. Aggregate per-block diffs via BatchBuilder.
        let mut builder = BatchBuilder::new();
        let mut block_count = 0u64;

        for block_hash in batch.blocks_iter() {
            // Convert Hash (Buf32) → B256 for the StateDiffProvider interface.
            let b256 = B256::from(block_hash.0);

            match self.state_diff_provider.get_state_diff_by_hash(b256) {
                Ok(Some(block_diff)) => {
                    builder.apply_block(&block_diff);
                    block_count += 1;
                }
                Ok(None) => {
                    debug!(?block_hash, "no state diff for block, skipping");
                }
                Err(e) => {
                    warn!(?block_hash, error = %e, "failed to fetch state diff for block");
                    return Err(eyre::eyre!(
                        "failed to fetch state diff for block {block_hash:?}: {e}"
                    ));
                }
            }
        }

        // 3. Build the aggregate diff.
        let batch_diff: BatchStateDiff = builder.build();

        if batch_diff.is_empty() {
            debug!(?batch_id, "batch has no state changes, returning None");
            return Ok(None);
        }

        // 4. Encode via strata-codec.
        let encoded = encode_to_vec(&batch_diff)?;

        info!(
            ?batch_id,
            block_count,
            encoded_len = encoded.len(),
            "encoded batch state diff for DA"
        );

        Ok(Some(encoded))
    }
}

/// Maps [`BatchId`] to the chunked envelope index assigned on submission.
type BatchIndex = HashMap<BatchId, u64>;

/// [`BatchDaProvider`] that posts DA via chunked envelope inscription.
pub(crate) struct ChunkedEnvelopeDaProvider {
    blob_provider: Arc<dyn DaBlobProvider>,
    envelope_handle: Arc<ChunkedEnvelopeHandle>,
    broadcast_ops: Arc<BroadcastDbOps>,
    magic_bytes: [u8; 4],
    /// Tracks which envelope index was assigned to each batch.
    batch_map: Mutex<BatchIndex>,
}

impl ChunkedEnvelopeDaProvider {
    pub(crate) fn new(
        blob_provider: Arc<dyn DaBlobProvider>,
        envelope_handle: Arc<ChunkedEnvelopeHandle>,
        broadcast_ops: Arc<BroadcastDbOps>,
        magic_bytes: [u8; 4],
    ) -> Self {
        Self {
            blob_provider,
            envelope_handle,
            broadcast_ops,
            magic_bytes,
            batch_map: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl BatchDaProvider for ChunkedEnvelopeDaProvider {
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<()> {
        // Check if already submitted.
        if self.batch_map.lock().unwrap().contains_key(&batch_id) {
            debug!(?batch_id, "batch DA already submitted, skipping");
            return Ok(());
        }

        let blob = self
            .blob_provider
            .get_blob(batch_id)?
            .ok_or_else(|| eyre::eyre!("no blob for batch {batch_id:?}"))?;

        let chunks = prepare_da_chunks(&blob)?;
        ensure!(!chunks.is_empty(), "prepare_da_chunks returned empty");

        let ops = self.envelope_handle.ops();

        // Determine the prev_tail_wtxid from the preceding envelope entry.
        let next_idx = ops.get_next_chunked_envelope_idx_async().await?;
        let prev_tail_wtxid = if next_idx == 0 {
            Buf32::zero()
        } else {
            ops.get_chunked_envelope_entry_async(next_idx - 1)
                .await?
                .map(|e| e.tail_wtxid())
                .unwrap_or(Buf32::zero())
        };

        let entry = ChunkedEnvelopeEntry::new_unsigned(chunks, self.magic_bytes, prev_tail_wtxid);

        let idx = self
            .envelope_handle
            .submit_entry(entry)
            .await
            .map_err(|e| eyre::eyre!("failed to submit envelope entry: {e}"))?;

        self.batch_map.lock().unwrap().insert(batch_id, idx);
        info!(?batch_id, %idx, "submitted chunked envelope for batch DA");
        Ok(())
    }

    async fn check_da_status(&self, batch_id: BatchId) -> eyre::Result<DaStatus> {
        let idx = {
            let map = self.batch_map.lock().unwrap();
            match map.get(&batch_id) {
                Some(&idx) => idx,
                None => return Ok(DaStatus::NotRequested),
            }
        };

        let entry = self
            .envelope_handle
            .ops()
            .get_chunked_envelope_entry_async(idx)
            .await?;
        let Some(entry) = entry else {
            bail!("envelope entry {idx} missing from DB for batch {batch_id:?}");
        };

        match entry.status {
            ChunkedEnvelopeStatus::Finalized => {
                let block_refs = self.build_da_block_refs(&entry).await?;
                Ok(DaStatus::Ready(block_refs))
            }
            ChunkedEnvelopeStatus::Unsigned
            | ChunkedEnvelopeStatus::NeedsResign
            | ChunkedEnvelopeStatus::Unpublished
            | ChunkedEnvelopeStatus::Published
            | ChunkedEnvelopeStatus::Confirmed => Ok(DaStatus::Pending),
        }
    }
}

impl ChunkedEnvelopeDaProvider {
    /// Builds [`L1DaBlockRef`] entries from broadcast DB for a finalized envelope.
    ///
    /// Collects the commit tx and all reveal txs, looks up each in the broadcast
    /// DB to get its L1 block, then groups by block into [`L1DaBlockRef`] entries.
    async fn build_da_block_refs(
        &self,
        entry: &ChunkedEnvelopeEntry,
    ) -> eyre::Result<Vec<L1DaBlockRef>> {
        // Collect all (txid, wtxid) pairs: commit + reveals.
        // The commit tx has no separate wtxid stored, so we use Buf32::zero() as
        // a placeholder — the commit tx carries no witness data of interest.
        let mut tx_pairs: Vec<(Buf32, Buf32)> = Vec::with_capacity(1 + entry.reveals.len());
        tx_pairs.push((entry.commit_txid, Buf32::zero()));
        for reveal in &entry.reveals {
            tx_pairs.push((reveal.txid, reveal.wtxid));
        }

        // Group by (block_hash, block_height) → Vec<(Txid, Wtxid)>.
        let mut block_map: HashMap<(Buf32, u64), Vec<(Txid, Wtxid)>> = HashMap::new();

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
                .push((Txid::from(*txid_buf), Wtxid::from(*wtxid_buf)));
        }

        // Build sorted L1DaBlockRef list (ascending by block height).
        let mut refs: Vec<L1DaBlockRef> = block_map
            .into_iter()
            .map(|((hash, height), txns)| {
                let commitment = L1BlockCommitment::from_height_u64(height, L1BlockId::from(hash))
                    .expect("valid block height");
                L1DaBlockRef::new(commitment, txns)
            })
            .collect();
        refs.sort_by_key(|r| r.block.height_u64());

        Ok(refs)
    }
}

