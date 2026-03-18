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
        );
        entry.status = status;
        entry.commit_txid = Buf32::from([0x11; 32]);
        entry.reveals = heights
            .iter()
            .enumerate()
            .map(|(i, _)| RevealTxMeta {
                vout_index: i as u32,
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

    /// Ensures finalized reveal transactions are grouped into sorted
    /// [`L1DaBlockRef`] values by their finalized L1 block height.
    #[tokio::test]
    async fn test_check_da_status_finalized_returns_sorted_refs() {
        let (provider, chunked_ops, broadcast_ops) = make_provider();
        let entry = make_entry(ChunkedEnvelopeStatus::Finalized, &[101, 100]);

        chunked_ops
            .put_chunked_envelope_entry_async(0, entry.clone())
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

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].block.height(), 100);
        assert_eq!(refs[1].block.height(), 101);
        assert_eq!(
            refs[0].txns,
            vec![(
                entry.reveals[1].txid.to_txid(),
                entry.reveals[1].wtxid.to_wtxid()
            )]
        );
        assert_eq!(
            refs[1].txns,
            vec![(
                entry.reveals[0].txid.to_txid(),
                entry.reveals[0].wtxid.to_wtxid()
            )]
        );
    }
}
