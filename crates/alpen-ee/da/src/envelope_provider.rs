//! [`BatchDaProvider`] implementation using chunked envelope inscription.

use std::{collections::HashMap, fmt, sync::Arc};

use alpen_ee_common::{
    prepare_da_chunks, BatchDaProvider, BatchId, DaBlobSource, DaStatus, L1DaBlockRef,
    DA_BLOB_VERSION,
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

/// Per-block accumulator: commit (when present) plus reveals tagged with
/// their commit-output vout for stable ordering.
#[derive(Default)]
struct BlockTxs {
    /// `Some` only on the L1 block where the commit finalized.
    commit: Option<FinalizedTxRef>,
    /// Reveals finalized in this block, with the vout of the commit output
    /// they spend (used to canonicalize ordering before producing `txns`).
    reveals: Vec<FinalizedRevealTx>,
}

struct FinalizedTxRef {
    txid: Txid,
    wtxid: Wtxid,
}

impl FinalizedTxRef {
    fn new(txid: Txid, wtxid: Wtxid) -> Self {
        Self { txid, wtxid }
    }

    fn into_pair(self) -> (Txid, Wtxid) {
        (self.txid, self.wtxid)
    }
}

struct FinalizedRevealTx {
    vout_index: u32,
    tx: FinalizedTxRef,
}

/// Groups commit + reveal txs by L1 block for [`L1DaBlockRef`] construction.
type BlockMap = HashMap<(Buf32, L1Height), BlockTxs>;

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

#[async_trait]
impl BatchDaProvider for ChunkedEnvelopeDaProvider {
    async fn post_batch_da(&self, batch_id: BatchId) -> eyre::Result<u64> {
        let blob = self.blob_provider.get_blob(batch_id).await?;
        let chunks = prepare_da_chunks(&blob)?;
        ensure!(!chunks.is_empty(), "prepare_da_chunks returned empty");

        let entry = ChunkedEnvelopeEntry::new_unsigned(chunks, self.magic_bytes, DA_BLOB_VERSION);
        let chunk_count = entry.chunk_data.len();

        let idx = self
            .envelope_handle
            .submit_entry(entry)
            .await
            .map_err(|e| eyre::eyre!("failed to submit envelope entry: {e}"))?;

        info!(
            ?batch_id,
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

        // Keep shared correlation fields on the span so status logs stay concise.
        let check_da_status_span = info_span!(
            "alpen_ee_check_da_status",
            ?batch_id,
            %envelope_idx,
            %entry,
        );

        async {
            debug!(status = %entry.status, "checking chunked envelope status");

            match entry.status {
                // `DaStatus::Ready` also updates the persistent EE DA bytecode
                // filter, which has no L1 reorg rollback path. Keep readiness
                // gated on reorg-safe finality so later batches do not skip
                // bytecode whose publishing tx could still be reorged out.
                ChunkedEnvelopeStatus::Finalized => {
                    let block_refs = self.build_da_block_refs(&entry).await?;
                    let da_block_refs = block_refs
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    info!(%da_block_refs, "batch DA finalized on L1");
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
    /// Collects the commit tx and all reveal txs and groups them by the L1
    /// block where each reached reorg-safe finality. The commit's OP_RETURN
    /// carries the EE DA magic/version marker; reveal tapscripts carry chunk
    /// payloads.
    async fn build_da_block_refs(
        &self,
        entry: &ChunkedEnvelopeEntry,
    ) -> eyre::Result<Vec<L1DaBlockRef>> {
        let mut block_map: BlockMap = HashMap::new();

        // Commit tx (carries the EE DA magic/version OP_RETURN).
        let (commit_block_hash, commit_block_height) =
            self.lookup_finalized(entry.commit_txid).await?;
        let commit_tx =
            FinalizedTxRef::new(entry.commit_txid.to_txid(), entry.commit_wtxid.to_wtxid());
        block_map
            .entry((commit_block_hash, commit_block_height))
            .or_default()
            .commit = Some(commit_tx);

        // Reveal txs (carry chunk payloads in their tapscript witnesses).
        for reveal in &entry.reveals {
            let (block_hash, block_height) = self.lookup_finalized(reveal.txid).await?;
            block_map
                .entry((block_hash, block_height))
                .or_default()
                .reveals
                .push(FinalizedRevealTx {
                    vout_index: reveal.vout_index,
                    tx: FinalizedTxRef::new(reveal.txid.to_txid(), reveal.wtxid.to_wtxid()),
                });
        }

        // Collapse each block's accumulated commit + reveals into a flat
        // ordered `txns` list. Within a block, the commit (if present) goes
        // first; reveals follow in ascending vout order.
        let mut refs: Vec<L1DaBlockRef> = block_map
            .into_iter()
            .map(|((hash, height), mut txs)| {
                txs.reveals.sort_by_key(|reveal| reveal.vout_index);
                let mut txns: Vec<(Txid, Wtxid)> =
                    Vec::with_capacity(txs.commit.is_some() as usize + txs.reveals.len());
                if let Some(commit) = txs.commit {
                    txns.push(commit.into_pair());
                }
                txns.extend(txs.reveals.into_iter().map(|reveal| reveal.tx.into_pair()));
                let commitment = L1BlockCommitment::new(height, L1BlockId::from(hash));
                L1DaBlockRef::new(commitment, txns)
            })
            .collect();
        refs.sort_by_key(|r| r.block.height());

        Ok(refs)
    }

    /// Looks up a tx in the broadcast DB and returns the finalized L1 block.
    async fn lookup_finalized(&self, txid: Buf32) -> eyre::Result<(Buf32, L1Height)> {
        let Some(tx_entry) = self.broadcast_ops.get_tx_entry_by_id_async(txid).await? else {
            bail!("broadcast entry for txid {txid} not found");
        };
        match tx_entry.status {
            L1TxStatus::Finalized {
                block_hash,
                block_height,
                ..
            } => Ok((block_hash, block_height)),
            other => bail!("expected Finalized status for txid {txid}, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::DaBlob;
    use async_trait::async_trait;
    use bitcoin::{
        absolute::LockTime, consensus::encode::serialize as btc_serialize, hashes::Hash,
        transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use strata_btcio::writer::chunked_envelope::ChunkedEnvelopeHandle;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{
        traits::DatabaseBackend,
        types::{L1TxEntry, RevealTxMeta},
    };
    use strata_l1_txfmt::MagicBytes;
    use strata_storage::ops::{
        chunked_envelope::{ChunkedEnvelopeOps, Context as ChunkedEnvelopeContext},
        l1tx_broadcast::{BroadcastDbOps, Context as BroadcastContext},
    };

    use super::*;

    struct NeverCalledBlobSource;

    #[async_trait]
    impl DaBlobSource for NeverCalledBlobSource {
        async fn get_blob(&self, _batch_id: BatchId) -> eyre::Result<DaBlob> {
            unreachable!("blob source is not used by check_da_status tests")
        }

        async fn are_state_diffs_ready(&self, _batch_id: BatchId) -> bool {
            unreachable!("blob source is not used by check_da_status tests")
        }
    }

    fn test_batch_id() -> BatchId {
        BatchId::from_parts(Default::default(), Default::default())
    }

    fn make_test_tx() -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::all_zeros(),
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                witness: Witness::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    fn make_entry(status: ChunkedEnvelopeStatus, heights: &[u64]) -> ChunkedEnvelopeEntry {
        let mut entry = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0xAA; 100]; heights.len().max(1)],
            MagicBytes::new([0x01, 0x02, 0x03, 0x04]),
            DA_BLOB_VERSION,
        );
        entry.status = status;
        entry.commit_txid = Buf32::from([0x11; 32]);
        entry.commit_wtxid = Buf32::from([0x12; 32]);
        entry.reveals = heights
            .iter()
            .enumerate()
            .map(|(i, _)| RevealTxMeta {
                vout_index: (i + 1) as u32,
                txid: Buf32::from([(0x20 + i as u8); 32]),
                wtxid: Buf32::from([(0x30 + i as u8); 32]),
                tx_bytes: btc_serialize(&make_test_tx()),
            })
            .collect();
        entry
    }

    fn make_provider() -> (
        ChunkedEnvelopeDaProvider,
        Arc<ChunkedEnvelopeOps>,
        Arc<BroadcastDbOps>,
    ) {
        let backend = get_test_sled_backend();
        let chunked_ops = Arc::new(
            ChunkedEnvelopeContext::new(backend.chunked_envelope_db())
                .into_ops(threadpool::Builder::new().num_threads(2).build()),
        );
        let broadcast_ops = Arc::new(
            BroadcastContext::new(backend.broadcast_db())
                .into_ops(threadpool::Builder::new().num_threads(2).build()),
        );
        let provider = ChunkedEnvelopeDaProvider::new(
            Arc::new(NeverCalledBlobSource),
            Arc::new(ChunkedEnvelopeHandle::new(chunked_ops.clone())),
            broadcast_ops.clone(),
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );

        (provider, chunked_ops, broadcast_ops)
    }

    fn finalized_tx_entry(height: u32) -> L1TxEntry {
        let mut entry = L1TxEntry::from_tx(&make_test_tx());
        entry.status = L1TxStatus::Finalized {
            confirmations: 6,
            block_hash: Buf32::from([height as u8; 32]),
            block_height: height,
        };
        entry
    }

    /// Ensures a persisted `envelope_idx` is treated as required state, not as
    /// an implicit "not requested yet" case.
    #[tokio::test]
    async fn test_check_da_status_errors_when_requested_entry_is_missing() {
        let (provider, _, _) = make_provider();

        let err = provider
            .check_da_status(test_batch_id(), 42)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("envelope entry 42 missing"));
    }

    /// Ensures DA status is determined from the specific persisted
    /// `envelope_idx`, even if later envelopes have already finalized.
    #[tokio::test]
    async fn test_check_da_status_uses_requested_envelope_idx() {
        let (provider, chunked_ops, _) = make_provider();

        chunked_ops
            .put_chunked_envelope_entry_async(
                0,
                make_entry(ChunkedEnvelopeStatus::Published, &[100]),
            )
            .await
            .unwrap();
        chunked_ops
            .put_chunked_envelope_entry_async(
                1,
                make_entry(ChunkedEnvelopeStatus::Finalized, &[101]),
            )
            .await
            .unwrap();

        let status = provider.check_da_status(test_batch_id(), 0).await.unwrap();
        assert!(matches!(status, DaStatus::Pending));
    }

    /// Ensures finalized commit + reveal transactions are grouped into sorted
    /// [`L1DaBlockRef`] values by their finalized L1 block height. Each block
    /// ref carries the commit (if it landed there) and the reveals.
    #[tokio::test]
    async fn test_check_da_status_finalized_returns_sorted_refs() {
        let (provider, chunked_ops, broadcast_ops) = make_provider();
        let entry = make_entry(ChunkedEnvelopeStatus::Finalized, &[101, 100]);

        chunked_ops
            .put_chunked_envelope_entry_async(0, entry.clone())
            .await
            .unwrap();
        // Commit landed at height 99 (before either reveal).
        broadcast_ops
            .put_tx_entry_async(entry.commit_txid, finalized_tx_entry(99))
            .await
            .unwrap();
        broadcast_ops
            .put_tx_entry_async(entry.reveals[0].txid, finalized_tx_entry(101))
            .await
            .unwrap();
        broadcast_ops
            .put_tx_entry_async(entry.reveals[1].txid, finalized_tx_entry(100))
            .await
            .unwrap();

        let status = provider.check_da_status(test_batch_id(), 0).await.unwrap();
        let DaStatus::Ready(refs) = status else {
            panic!("expected finalized envelope to be ready");
        };

        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].block.height(), 99);
        assert_eq!(refs[1].block.height(), 100);
        assert_eq!(refs[2].block.height(), 101);

        // Block 99 holds only the commit tx.
        assert_eq!(
            refs[0].txns,
            vec![(entry.commit_txid.to_txid(), entry.commit_wtxid.to_wtxid())]
        );

        // Blocks 100 and 101 each hold one reveal tx.
        assert_eq!(
            refs[1].txns,
            vec![(
                entry.reveals[1].txid.to_txid(),
                entry.reveals[1].wtxid.to_wtxid()
            )]
        );
        assert_eq!(
            refs[2].txns,
            vec![(
                entry.reveals[0].txid.to_txid(),
                entry.reveals[0].wtxid.to_wtxid()
            )]
        );
    }
}
