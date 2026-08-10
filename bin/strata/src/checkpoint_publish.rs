//! Checkpoint-aware policy evaluated at the final Bitcoin publication boundary.

use std::sync::Arc;

use bitcoin::Transaction;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::{SectionStateExt, Subprotocol};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_btc_types::TxidExt;
use strata_btcio::broadcaster::{PublishDecision, PublishPolicy};
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_db_types::{
    DbResult,
    backend::DatabaseBackend,
    l1_broadcast::{L1BroadcastDatabase, L1TxStatus},
    l1_writer::{BundledPayloadEntry, L1BundleStatus, L1WriterDatabase},
};
use strata_identifiers::Epoch;
use strata_storage::NodeStorage;
use tracing::warn;

pub(crate) struct CheckpointPublishPolicy {
    storage: Arc<NodeStorage>,
}

impl CheckpointPublishPolicy {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    fn decide_checkpoint(&self, checkpoint: &CheckpointPayload) -> PublishDecision {
        let latest_state = match self.storage.fetch_canonical_asm_state_blocking() {
            Ok(Some((_, state))) => state,
            Ok(None) => return PublishDecision::Defer,
            Err(err) => {
                warn!(%err, "deferring checkpoint while canonical ASM state is unavailable");
                return PublishDecision::Defer;
            }
        };
        let Some(latest_epoch) = latest_state
            .state()
            .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
            .and_then(|section| section.try_to_state::<CheckpointSubprotocol>().ok())
            .map(|state| state.verified_tip().epoch)
        else {
            return PublishDecision::Defer;
        };
        if checkpoint.new_tip().epoch > latest_epoch {
            return PublishDecision::Publish;
        }

        let safe_epoch = self
            .storage
            .client_state()
            .fetch_most_recent_state()
            .ok()
            .flatten()
            .filter(|(block, _)| {
                self.storage
                    .l1()
                    .get_canonical_blockid_at_height(block.height())
                    .ok()
                    .flatten()
                    == Some(*block.blkid())
            })
            .and_then(|(_, state)| state.get_declared_final_epoch())
            .map(|commitment| commitment.epoch());
        checkpoint_decision(checkpoint.new_tip().epoch, latest_epoch, safe_epoch)
    }

    fn decide_bundle(&self, bundle: &BundledPayloadEntry, tx: &Transaction) -> PublishDecision {
        if bundle.status == L1BundleStatus::Abandoned {
            return PublishDecision::Abandon;
        }
        let checkpoint = match checkpoint_from_payload(&bundle.payload) {
            Ok(Some(checkpoint)) => checkpoint,
            Ok(None) => return PublishDecision::Publish,
            Err(()) => return PublishDecision::Defer,
        };
        let db = self.storage.db().broadcast_db();
        let status = |txid: [u8; 32]| {
            db.get_tx_entry_by_id(txid.into())
                .map(|entry| entry.map(|entry| entry.status))
        };
        let (commit, reveal) = match (status(bundle.commit_txid.0), status(bundle.reveal_txid.0)) {
            (Ok(commit), Ok(reveal)) => (commit, reveal),
            (Err(err), _) | (_, Err(err)) => {
                warn!(%err, "deferring checkpoint while its publication pair is unavailable");
                return PublishDecision::Defer;
            }
        };
        let txid = tx.compute_txid().to_buf32().0;
        let member = if txid == bundle.commit_txid.0 {
            PairMember::Commit
        } else {
            PairMember::Reveal
        };
        pair_decision(self.decide_checkpoint(&checkpoint), member, commit, reveal)
    }
}

impl PublishPolicy for CheckpointPublishPolicy {
    fn decide(&self, tx: &Transaction) -> PublishDecision {
        match writer_bundle_for_tx(self.storage.db().as_ref(), tx) {
            Ok(Some(bundle)) => self.decide_bundle(&bundle, tx),
            Ok(None) => PublishDecision::Publish,
            Err(err) => {
                warn!(%err, "deferring transaction while writer queue is unavailable");
                PublishDecision::Defer
            }
        }
    }
}

pub(crate) fn checkpoint_from_payload(
    payload: &L1Payload,
) -> Result<Option<CheckpointPayload>, ()> {
    let tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
    if payload.tag().subproto_id() != tag.subproto_id()
        || payload.tag().tx_type() != tag.tx_type()
        || payload.tag().aux_data() != tag.aux_data()
    {
        return Ok(None);
    }
    decode_buf_exact::<CodecSsz<CheckpointPayload>>(&payload.data().concat())
        .map(CodecSsz::into_inner)
        .map(Some)
        .map_err(|_| ())
}

fn writer_bundle_for_tx(
    db: &impl DatabaseBackend,
    tx: &Transaction,
) -> DbResult<Option<BundledPayloadEntry>> {
    let txid = tx.compute_txid().to_buf32().0;
    let writer = db.writer_db();
    for idx in (0..writer.get_next_payload_idx()?).rev() {
        let Some(entry) = writer.get_payload_entry_by_idx(idx)? else {
            continue;
        };
        if entry.commit_txid.0 == txid || entry.reveal_txid.0 == txid {
            return Ok(Some(entry));
        }
        if matches!(entry.status, L1BundleStatus::Finalized) {
            break;
        }
    }
    Ok(None)
}

fn checkpoint_decision(
    checkpoint_epoch: Epoch,
    latest_epoch: Epoch,
    safe_epoch: Option<Epoch>,
) -> PublishDecision {
    if safe_epoch.is_some_and(|epoch| checkpoint_epoch <= epoch) {
        PublishDecision::Abandon
    } else if checkpoint_epoch > latest_epoch {
        PublishDecision::Publish
    } else {
        PublishDecision::Defer
    }
}

#[derive(Clone, Copy)]
enum PairMember {
    Commit,
    Reveal,
}

fn pair_decision(
    decision: PublishDecision,
    member: PairMember,
    commit: Option<L1TxStatus>,
    reveal: Option<L1TxStatus>,
) -> PublishDecision {
    let reached_l1 = |status: &Option<L1TxStatus>| {
        matches!(
            status,
            Some(
                L1TxStatus::Published | L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
            )
        )
    };
    if reached_l1(&commit) || reached_l1(&reveal) {
        return PublishDecision::Publish;
    }
    if matches!(member, PairMember::Reveal)
        && matches!(commit, None | Some(L1TxStatus::Unpublished))
    {
        return PublishDecision::Publish;
    }
    if matches!(member, PairMember::Commit) && reveal.is_none() {
        return PublishDecision::Defer;
    }
    if decision == PublishDecision::Abandon
        && matches!(member, PairMember::Commit)
        && matches!(commit, None | Some(L1TxStatus::Unpublished))
        && matches!(reveal, None | Some(L1TxStatus::Unpublished))
    {
        return PublishDecision::Defer;
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_and_pair_decisions_cover_safe_publication_states() {
        let epoch = Epoch::from(14u32);
        assert_eq!(
            checkpoint_decision(epoch, Epoch::from(13u32), None),
            PublishDecision::Publish
        );
        assert_eq!(
            checkpoint_decision(epoch, epoch, None),
            PublishDecision::Defer
        );
        assert_eq!(
            checkpoint_decision(epoch, epoch, Some(epoch)),
            PublishDecision::Abandon
        );
        assert_eq!(
            checkpoint_decision(epoch, Epoch::from(13u32), Some(epoch)),
            PublishDecision::Abandon
        );

        for (member, commit, reveal, expected) in [
            (
                PairMember::Commit,
                Some(L1TxStatus::Unpublished),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Defer,
            ),
            (
                PairMember::Reveal,
                Some(L1TxStatus::Unpublished),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Publish,
            ),
            (
                PairMember::Commit,
                Some(L1TxStatus::Unpublished),
                Some(L1TxStatus::InvalidInputs),
                PublishDecision::Abandon,
            ),
            (
                PairMember::Reveal,
                Some(L1TxStatus::Published),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Publish,
            ),
        ] {
            assert_eq!(
                pair_decision(PublishDecision::Abandon, member, commit, reveal),
                expected
            );
        }
    }
}
