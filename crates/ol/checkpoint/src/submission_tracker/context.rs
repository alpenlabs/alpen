//! I/O the submission tracker needs: writer bundles, broadcaster status, L1 chain, persistence.

use std::sync::Arc;

use anyhow::Context;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_btcio::broadcaster::L1BroadcastHandle;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_db_types::common::L1TxId;
use strata_db_types::l1_broadcast::L1TxStatus;
use strata_db_types::l1_writer::BundleIdx;
use strata_db_types::ol_checkpoint::RejectedCheckpointEntry;
use strata_identifiers::{Buf32, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height};
use strata_storage::NodeStorage;
use tokio::runtime::Handle;
use tracing::warn;

/// A checkpoint the sequencer handed to the L1 writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CheckpointBundle {
    /// Epoch commitment the checkpoint declares.
    pub(crate) commitment: EpochCommitment,

    /// Reveal transaction carrying the envelope, `None` until the writer signs the bundle.
    pub(crate) reveal_txid: Option<L1TxId>,
}

/// Broadcaster view of a reveal transaction after following RBF replacements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RevealStatus {
    /// Transaction that is live for the reveal, which differs from the bundle's after a
    /// replacement.
    pub(crate) txid: L1TxId,

    /// Block the broadcaster saw the transaction in, if it is confirmed.
    pub(crate) inclusion: Option<L1BlockCommitment>,
}

/// Operations the submission tracker delegates to storage and the broadcaster.
///
/// Kept as a trait so the tracker's logic is tested against an in-memory stub.
pub(crate) trait SubmissionTrackerContext: Send + Sync + 'static {
    /// First writer bundle index the tracker has not settled, if it ever committed a scan.
    fn scan_cursor(&self) -> anyhow::Result<Option<BundleIdx>>;

    /// Rejections recorded so far.
    fn rejected_checkpoints(&self) -> anyhow::Result<Vec<RejectedCheckpointEntry>>;

    /// Atomically stores the scan cursor and newly recorded rejections.
    fn commit_scan(
        &self,
        cursor: BundleIdx,
        rejected: Vec<RejectedCheckpointEntry>,
    ) -> anyhow::Result<()>;

    /// Index the writer will assign to its next bundle.
    fn next_bundle_idx(&self) -> anyhow::Result<BundleIdx>;

    /// Returns the checkpoint in bundle `idx`, or `None` if the bundle carries something else.
    fn checkpoint_bundle(&self, idx: BundleIdx) -> anyhow::Result<Option<CheckpointBundle>>;

    /// Resolves a reveal transaction through the broadcaster, or `None` if it is unknown there.
    fn resolve_reveal(&self, txid: L1TxId) -> anyhow::Result<Option<RevealStatus>>;

    /// Canonical L1 block at `height`, if the node has one there.
    fn canonical_l1_block(&self, height: L1Height) -> anyhow::Result<Option<L1BlockId>>;
}

/// [`SubmissionTrackerContext`] backed by node storage and the running L1 broadcaster.
pub(crate) struct SubmissionTrackerContextImpl {
    storage: Arc<NodeStorage>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    runtime: Handle,
}

impl SubmissionTrackerContextImpl {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        runtime: Handle,
    ) -> Self {
        Self {
            storage,
            broadcast_handle,
            runtime,
        }
    }
}

impl SubmissionTrackerContext for SubmissionTrackerContextImpl {
    fn scan_cursor(&self) -> anyhow::Result<Option<BundleIdx>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_submission_cursor_blocking()?)
    }

    fn rejected_checkpoints(&self) -> anyhow::Result<Vec<RejectedCheckpointEntry>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_rejected_checkpoints_blocking()?)
    }

    fn commit_scan(
        &self,
        cursor: BundleIdx,
        rejected: Vec<RejectedCheckpointEntry>,
    ) -> anyhow::Result<()> {
        Ok(self
            .storage
            .ol_checkpoint()
            .put_checkpoint_submission_scan_blocking(cursor, rejected)?)
    }

    fn next_bundle_idx(&self) -> anyhow::Result<BundleIdx> {
        Ok(self.storage.l1_writer().get_next_payload_idx_blocking()?)
    }

    fn checkpoint_bundle(&self, idx: BundleIdx) -> anyhow::Result<Option<CheckpointBundle>> {
        let entry = self
            .storage
            .l1_writer()
            .get_payload_entry_by_idx_blocking(idx)?
            .with_context(|| format!("L1 writer bundle {idx} is missing"))?;
        if *entry.payload.tag() != *OL_STF_CHECKPOINT_TX_TAG {
            return Ok(None);
        }

        // The sequencer writes one `CodecSsz<CheckpointPayload>` chunk, the same encoding the
        // ASM decodes from the envelope.
        let decoded = entry
            .payload
            .data()
            .next()
            .map(decode_buf_exact::<CodecSsz<CheckpointPayload>>);
        let Some(Ok(payload)) = decoded else {
            warn!(
                bundle_idx = idx,
                "skipping checkpoint bundle with an undecodable payload"
            );
            return Ok(None);
        };
        let payload = payload.into_inner();
        let tip = payload.new_tip();
        let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());

        // The writer leaves the txids zeroed until it signs the bundle.
        let reveal_txid = (entry.reveal_txid != L1TxId::zero()).then_some(entry.reveal_txid);
        Ok(Some(CheckpointBundle {
            commitment,
            reveal_txid,
        }))
    }

    fn resolve_reveal(&self, txid: L1TxId) -> anyhow::Result<Option<RevealStatus>> {
        let resolved = self.runtime.block_on(
            self.broadcast_handle
                .get_active_tx_entry_by_id_async(Buf32(txid.0)),
        )?;
        Ok(resolved.map(|(live_txid, entry)| RevealStatus {
            txid: L1TxId::from(live_txid.0),
            inclusion: inclusion_block(&entry.status),
        }))
    }

    fn canonical_l1_block(&self, height: L1Height) -> anyhow::Result<Option<L1BlockId>> {
        Ok(self.storage.l1().get_canonical_blockid_at_height(height)?)
    }
}

/// Block a transaction was included in, per the broadcaster.
fn inclusion_block(status: &L1TxStatus) -> Option<L1BlockCommitment> {
    match status {
        L1TxStatus::Confirmed {
            block_hash,
            block_height,
            ..
        }
        | L1TxStatus::Finalized {
            block_hash,
            block_height,
            ..
        } => Some(L1BlockCommitment::new(
            *block_height,
            L1BlockId::from(*block_hash),
        )),
        _ => None,
    }
}
