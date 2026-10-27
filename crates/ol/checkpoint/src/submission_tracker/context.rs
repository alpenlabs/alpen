//! I/O the submission tracker needs: writer bundles, broadcaster status, L1 chain, persistence.

use std::sync::Arc;

use anyhow::Context;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_btcio::broadcaster::L1BroadcastHandle;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::common::L1TxId;
use strata_db_types::l1_broadcast::L1TxStatus;
use strata_db_types::l1_writer::BundleIdx;
use strata_db_types::ol_checkpoint::RejectedCheckpointEntry;
use strata_identifiers::{Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height};
use strata_storage::NodeStorage;
use thiserror::Error;
use tokio::runtime::Handle;

/// What an L1 writer bundle carries, as far as the tracker is concerned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BundleContents {
    /// Something other than a checkpoint.
    Other,

    /// A checkpoint-tagged payload that does not decode, which only a bug or corruption leaves.
    Undecodable(UndecodablePayload),

    /// A checkpoint.
    Checkpoint(CheckpointBundle),
}

/// Why a checkpoint-tagged writer payload does not decode.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub(crate) enum UndecodablePayload {
    /// The payload has no data chunk.
    #[error("payload has no data chunk")]
    MissingChunk,

    /// The first chunk is not a checkpoint payload; holds the codec error's message.
    #[error("checkpoint payload does not decode: {0}")]
    Decode(String),
}

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
    fn get_scan_cursor(&self) -> anyhow::Result<Option<BundleIdx>>;

    /// Rejections recorded so far.
    fn get_rejected_checkpoints(&self) -> anyhow::Result<Vec<RejectedCheckpointEntry>>;

    /// Atomically stores the scan cursor and newly recorded rejections.
    fn commit_scan(
        &self,
        cursor: BundleIdx,
        rejected: Vec<RejectedCheckpointEntry>,
    ) -> anyhow::Result<()>;

    /// Index the writer will assign to its next bundle.
    fn get_next_bundle_idx(&self) -> anyhow::Result<BundleIdx>;

    /// Returns what bundle `idx` carries.
    fn get_bundle_contents(&self, idx: BundleIdx) -> anyhow::Result<BundleContents>;

    /// Resolves a reveal transaction through the broadcaster, or `None` if it is unknown there.
    fn resolve_reveal(&self, txid: L1TxId) -> anyhow::Result<Option<RevealStatus>>;

    /// L1 refs of the checkpoints the CSM observed the ASM accept for `epoch`.
    ///
    /// An L1 reorg can leave refs on abandoned blocks behind, so callers keep the canonical ones.
    fn get_checkpoint_l1_refs_for_epoch(
        &self,
        epoch: Epoch,
    ) -> anyhow::Result<Vec<(EpochCommitment, CheckpointL1Ref)>>;

    /// Canonical L1 block at `height`, if the node has one there.
    fn get_canonical_l1_blockid_at(&self, height: L1Height) -> anyhow::Result<Option<L1BlockId>>;
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
    fn get_scan_cursor(&self) -> anyhow::Result<Option<BundleIdx>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_submission_cursor_blocking()?)
    }

    fn get_rejected_checkpoints(&self) -> anyhow::Result<Vec<RejectedCheckpointEntry>> {
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

    fn get_next_bundle_idx(&self) -> anyhow::Result<BundleIdx> {
        Ok(self.storage.l1_writer().get_next_payload_idx_blocking()?)
    }

    fn get_bundle_contents(&self, idx: BundleIdx) -> anyhow::Result<BundleContents> {
        let entry = self
            .storage
            .l1_writer()
            .get_payload_entry_by_idx_blocking(idx)?
            .with_context(|| format!("L1 writer bundle {idx} is missing"))?;
        if *entry.payload.tag() != *OL_STF_CHECKPOINT_TX_TAG {
            return Ok(BundleContents::Other);
        }

        // The sequencer writes one `CodecSsz<CheckpointPayload>` chunk, the same encoding the
        // ASM decodes from the envelope.
        let Some(chunk) = entry.payload.data().next() else {
            return Ok(BundleContents::Undecodable(
                UndecodablePayload::MissingChunk,
            ));
        };
        let payload = match decode_buf_exact::<CodecSsz<CheckpointPayload>>(chunk) {
            Ok(payload) => payload.into_inner(),
            Err(err) => {
                return Ok(BundleContents::Undecodable(UndecodablePayload::Decode(
                    err.to_string(),
                )));
            }
        };
        let tip = payload.new_tip();
        let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());

        // The writer leaves the txids zeroed until it signs the bundle.
        let reveal_txid = (entry.reveal_txid != L1TxId::zero()).then_some(entry.reveal_txid);
        Ok(BundleContents::Checkpoint(CheckpointBundle {
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

    fn get_checkpoint_l1_refs_for_epoch(
        &self,
        epoch: Epoch,
    ) -> anyhow::Result<Vec<(EpochCommitment, CheckpointL1Ref)>> {
        let ol_checkpoint = self.storage.ol_checkpoint();
        let mut refs = Vec::new();
        for commitment in
            ol_checkpoint.get_observed_checkpoint_commitments_for_epoch_blocking(epoch)?
        {
            if let Some(l1_ref) = ol_checkpoint.get_checkpoint_l1_ref_blocking(commitment)? {
                refs.push((commitment, l1_ref));
            }
        }
        Ok(refs)
    }

    fn get_canonical_l1_blockid_at(&self, height: L1Height) -> anyhow::Result<Option<L1BlockId>> {
        // TODO(STR-4601): reverting the L1 chain purges this cache before deleting the rows, so
        // during a reorg this can briefly return a reverted block. It can't produce a false
        // rejection record short of a reorg as deep as the reorg-safe depth.
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
