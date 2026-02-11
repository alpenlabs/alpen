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

use std::{collections::HashMap, sync::Arc};

use alloy_primitives::B256;
use alpen_ee_common::{
    prepare_da_chunks, BatchDaProvider, BatchId, BatchStorage, DaBlob, DaBlobProvider, DaStatus,
    EvmHeaderDigest, HeaderDigestProvider, L1DaBlockRef,
};
use alpen_ee_database::BroadcastDbOps;
use alpen_reth_db::{EeDaContext, StateDiffProvider};
use alpen_reth_statediff::BatchBuilder;
use async_trait::async_trait;
use bitcoin::{Txid, Wtxid};
use eyre::{bail, ensure};
use strata_btcio::writer::chunked_envelope::ChunkedEnvelopeHandle;
use strata_db_types::types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxStatus};
use strata_identifiers::{L1BlockCommitment, L1BlockId};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::buf::Buf32;
use tracing::*;

/// Groups reveal txs by L1 block for [`L1DaBlockRef`] construction.
type BlockMap = HashMap<(Buf32, u64), Vec<(Txid, Wtxid)>>;

/// [`HeaderDigestProvider`] backed by a Reth [`HeaderProvider`](reth_provider::HeaderProvider).
pub(crate) struct RethHeaderDigestProvider<P> {
    provider: P,
}

impl<P> RethHeaderDigestProvider<P> {
    pub(crate) fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> HeaderDigestProvider for RethHeaderDigestProvider<P>
where
    P: reth_provider::HeaderProvider<Header = reth_primitives::Header> + Send + Sync,
{
    fn header_digest(&self, block_num: u64) -> eyre::Result<EvmHeaderDigest> {
        let header = self
            .provider
            .header_by_number(block_num)?
            .ok_or_else(|| eyre::eyre!("no header for block {block_num}"))?;
        Ok(EvmHeaderDigest {
            block_num: header.number,
            timestamp: header.timestamp,
            base_fee: header.base_fee_per_gas.ok_or_else(|| {
                eyre::eyre!(
                    "block {block_num} missing base_fee_per_gas; \
                     Alpen is post-London from genesis so this should always be present"
                )
            })?,
            gas_used: header.gas_used,
            gas_limit: header.gas_limit,
        })
    }
}

/// [`DaBlobProvider`] that builds encoded DA blobs from Reth state diffs.
///
/// For each batch, it:
/// 1. Retrieves the block range from [`BatchStorage`].
/// 2. Fetches per-block [`BlockStateChanges`](alpen_reth_statediff::BlockStateChanges) from the [`StateDiffProvider`].
/// 3. Aggregates them into a [`BatchStateDiff`](alpen_reth_statediff::BatchStateDiff) via [`BatchBuilder`].
/// 4. Reads the last block's header to build [`EvmHeaderDigest`].
/// 5. Returns the assembled [`DaBlob`].
pub(crate) struct StateDiffBlobProvider<S, D, H> {
    batch_storage: Arc<S>,
    state_diff_provider: Arc<D>,
    header_digest: Arc<H>,
    da_ctx: Arc<dyn EeDaContext + Send + Sync>,
}

impl<S, D, H> StateDiffBlobProvider<S, D, H> {
    pub(crate) fn new(
        batch_storage: Arc<S>,
        state_diff_provider: Arc<D>,
        header_digest: Arc<H>,
        da_ctx: Arc<dyn EeDaContext + Send + Sync>,
    ) -> Self {
        Self {
            batch_storage,
            state_diff_provider,
            header_digest,
            da_ctx,
        }
    }

    /// Returns `true` if the bytecode has not been published in a prior batch
    /// and therefore still needs to be included in the DA blob.
    ///
    /// On DB errors the bytecode is conservatively kept — duplicates are safe,
    /// missing data is not.
    fn bytecode_needs_publish(&self, code_hash: &B256) -> bool {
        match self.da_ctx.is_code_hash_published(code_hash) {
            Ok(published) => !published,
            Err(e) => {
                warn!(?code_hash, error = %e, "failed to check published status, keeping bytecode");
                true
            }
        }
    }
}

#[async_trait]
impl<S, D, H> DaBlobProvider for StateDiffBlobProvider<S, D, H>
where
    S: BatchStorage,
    D: StateDiffProvider + Send + Sync,
    H: HeaderDigestProvider,
{
    async fn get_blob(&self, batch_id: BatchId) -> eyre::Result<DaBlob> {
        // 1. Look up the batch to get its block range.
        let (batch, _status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await?
            .ok_or_else(|| eyre::eyre!("batch {batch_id:?} not found in storage"))?;

        // 2. Aggregate per-block diffs via BatchBuilder.
        let mut builder = BatchBuilder::new();
        let mut block_count = 0u64;

        for block_hash in batch.blocks_iter() {
            // Convert Hash (Buf32) -> B256 for the StateDiffProvider interface.
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

        // 3. Build the aggregate diff and filter already-published bytecodes.
        let mut state_diff = builder.build();

        let before = state_diff.deployed_bytecodes.len();
        state_diff
            .deployed_bytecodes
            .retain(|hash, _| self.bytecode_needs_publish(hash));
        let deduped = before - state_diff.deployed_bytecodes.len();

        info!(
            ?batch_id,
            block_count,
            is_empty = state_diff.is_empty(),
            deduped,
            "built DA blob for batch"
        );

        // 4. Read the last block's header for chain-reconstruction metadata.
        let evm_header = self.header_digest.header_digest(batch.last_blocknum())?;

        // 5. Construct the DaBlob with metadata.
        Ok(DaBlob {
            batch_id,
            evm_header,
            state_diff,
        })
    }

    async fn are_state_diffs_ready(&self, batch_id: BatchId) -> eyre::Result<bool> {
        // Look up the batch to get its block range.
        let (batch, _status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await?
            .ok_or_else(|| eyre::eyre!("batch {batch_id:?} not found in storage"))?;

        // Check if all blocks have state diffs available.
        for block_hash in batch.blocks_iter() {
            let b256 = B256::from(block_hash.0);
            match self.state_diff_provider.get_state_diff_by_hash(b256) {
                Ok(Some(_)) => {
                    // State diff exists for this block
                }
                Ok(None) => {
                    // State diff not yet available
                    debug!(?block_hash, "state diff not available for block");
                    return Ok(false);
                }
                Err(e) => {
                    warn!(?block_hash, error = %e, "failed to check state diff for block");
                    return Err(eyre::eyre!(
                        "failed to check state diff for block {block_hash:?}: {e}"
                    ));
                }
            }
        }

        Ok(true)
    }
}

/// [`BatchDaProvider`] that posts DA via chunked envelope inscription.
pub(crate) struct ChunkedEnvelopeDaProvider {
    blob_provider: Arc<dyn DaBlobProvider>,
    envelope_handle: Arc<ChunkedEnvelopeHandle>,
    broadcast_ops: Arc<BroadcastDbOps>,
    magic_bytes: MagicBytes,
}

impl ChunkedEnvelopeDaProvider {
    pub(crate) fn new(
        blob_provider: Arc<dyn DaBlobProvider>,
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

#[async_trait]
impl BatchDaProvider for ChunkedEnvelopeDaProvider {
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<u64> {
        let blob = self.blob_provider.get_blob(batch_id).await?;
        let chunks = prepare_da_chunks(&blob)?;
        ensure!(!chunks.is_empty(), "prepare_da_chunks returned empty");

        let entry = ChunkedEnvelopeEntry::new_unsigned(chunks, self.magic_bytes);

        let idx = self
            .envelope_handle
            .submit_entry(entry)
            .await
            .map_err(|e| eyre::eyre!("failed to submit envelope entry: {e}"))?;

        info!(?batch_id, %idx, "submitted chunked envelope for batch DA");
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

        match entry.status {
            ChunkedEnvelopeStatus::Finalized => {
                let block_refs = self.build_da_block_refs(&entry).await?;
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
