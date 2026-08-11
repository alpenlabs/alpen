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
    chunked_envelope::L1ChunkedEnvelopeDatabase,
    l1_broadcast::{L1BroadcastDatabase, L1TxStatus},
    l1_writer::{BundledPayloadEntry, L1BundleStatus, L1WriterDatabase},
};
use strata_identifiers::Epoch;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
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

        let safe_epoch = self.safe_checkpoint_epoch().unwrap_or_else(|err| {
            warn!(%err, "checkpoint safe epoch is unavailable");
            None
        });
        checkpoint_decision(checkpoint.new_tip().epoch, latest_epoch, safe_epoch)
    }

    /// Returns the final epoch from the latest client state when its L1 anchor is canonical.
    fn safe_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        let Some((block, state)) = self.storage.client_state().fetch_most_recent_state()? else {
            return Ok(None);
        };
        let canonical = self
            .storage
            .l1()
            .get_canonical_blockid_at_height(block.height())?;
        if canonical != Some(*block.blkid()) {
            return Ok(None);
        }
        Ok(state
            .get_declared_final_epoch()
            .map(|commitment| commitment.epoch()))
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
        match publication_for_tx(self.storage.db().as_ref(), tx) {
            Ok((Some(bundle), _)) => self.decide_bundle(&bundle, tx),
            Ok((None, decision)) => decision,
            Err(err) => {
                warn!(%err, "deferring transaction while publication linkage is unavailable");
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

fn publication_for_tx(
    db: &impl DatabaseBackend,
    tx: &Transaction,
) -> DbResult<(Option<BundledPayloadEntry>, PublishDecision)> {
    let txid = tx.compute_txid().to_buf32().0;

    if let Some(bundle) = find_writer_publication(db, txid)? {
        return Ok((Some(bundle), PublishDecision::Publish));
    }
    if is_chunked_publication(db, txid)? {
        return Ok((None, PublishDecision::Publish));
    }
    unlinked_pair_decision(db, tx, txid).map(|decision| (None, decision))
}

fn find_writer_publication(
    db: &impl DatabaseBackend,
    txid: [u8; 32],
) -> DbResult<Option<BundledPayloadEntry>> {
    let writer = db.writer_db();
    for idx in (0..writer.get_next_payload_idx()?).rev() {
        let Some(entry) = writer.get_payload_entry_by_idx(idx)? else {
            continue;
        };
        if entry.commit_txid.0 == txid || entry.reveal_txid.0 == txid {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn is_chunked_publication(db: &impl DatabaseBackend, txid: [u8; 32]) -> DbResult<bool> {
    let chunked = db.chunked_envelope_db();
    for idx in (0..chunked.get_next_chunked_envelope_idx()?).rev() {
        let Some(entry) = chunked.get_chunked_envelope_entry(idx)? else {
            continue;
        };
        if entry.commit_txid.0 == txid || entry.reveals.iter().any(|reveal| reveal.txid.0 == txid) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn unlinked_pair_decision(
    db: &impl DatabaseBackend,
    tx: &Transaction,
    txid: [u8; 32],
) -> DbResult<PublishDecision> {
    let broadcast = db.broadcast_db();
    let current = broadcast
        .get_tx_entry_by_id(txid.into())?
        .map(|entry| entry.status);
    if is_envelope_reveal(tx)
        && let Some(parent) = tx.input.first().map(|input| input.previous_output)
        && parent.vout == 0
        && let Some(parent) = broadcast.get_tx_entry_by_id(parent.txid.to_buf32())?
    {
        return Ok(pair_decision(
            PublishDecision::Abandon,
            PairMember::Reveal,
            Some(parent.status),
            current,
        ));
    }
    for idx in 0..broadcast.get_next_tx_idx()? {
        let Some(entry) = broadcast.get_tx_entry(idx)? else {
            continue;
        };
        let Ok(candidate) = entry.try_to_tx() else {
            continue;
        };
        if is_envelope_reveal(&candidate)
            && candidate.input.first().is_some_and(|input| {
                input.previous_output.txid.to_buf32().0 == txid && input.previous_output.vout == 0
            })
        {
            return Ok(pair_decision(
                PublishDecision::Abandon,
                PairMember::Commit,
                current,
                Some(entry.status),
            ));
        }
    }
    warn!(txid = %tx.compute_txid(), "deferring transaction without durable publication linkage");
    Ok(PublishDecision::Defer)
}

fn is_envelope_reveal(tx: &Transaction) -> bool {
    tx.input
        .first()
        .and_then(|input| input.witness.taproot_leaf_script())
        .is_some_and(|leaf| parse_envelope_payload(&leaf.script.into()).is_ok())
}

/// Publishes checkpoints ahead of ASM, abandons reorg-safe checkpoints, and
/// defers those accepted by canonical ASM but not yet safe.
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

/// Preserves commit/reveal recovery while applying the checkpoint decision.
///
/// If either transaction may already be on L1, the other is published to finish
/// the pair. A reveal is also published when its commit record is missing or
/// unpublished. This probes whether a crash occurred after the commit reached
/// bitcoind; a genuinely absent commit makes the reveal fail safely with invalid
/// inputs.
///
/// A commit is deferred until its reveal is durably recorded. When both records
/// exist but a stale pair is still unpublished, the commit remains deferred so
/// the reveal probe runs first. All other states use the checkpoint decision.
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
    use bitcoin::{OutPoint, Sequence, Transaction, TxIn, Witness, absolute, transaction};
    use strata_asm_proto_txs_test_utils::create_reveal_transaction_stub;
    use strata_btc_types::TxidExt;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{
        backend::DatabaseBackend,
        chunked_envelope::{ChunkedEnvelopeEntry, L1ChunkedEnvelopeDatabase},
        l1_broadcast::L1TxEntry,
    };
    use strata_l1_txfmt::MagicBytes;

    use super::*;

    #[test]
    fn unlinked_transaction_is_quarantined_but_chunked_transaction_is_known() {
        let db = get_test_sled_backend();
        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        assert_eq!(
            publication_for_tx(db.as_ref(), &tx).unwrap().1,
            PublishDecision::Defer
        );

        let mut chunked =
            ChunkedEnvelopeEntry::new_unsigned(vec![vec![1]], MagicBytes::new([1; 4]), 1);
        chunked.commit_txid = tx.compute_txid().to_buf32().0.into();
        db.chunked_envelope_db()
            .put_chunked_envelope_entry(0, chunked)
            .unwrap();
        assert_eq!(
            publication_for_tx(db.as_ref(), &tx).unwrap().1,
            PublishDecision::Publish
        );
    }

    #[test]
    fn unlinked_legacy_pair_probes_reveal_before_abandoning_commit() {
        let db = get_test_sled_backend();
        let funding = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        let commit = Transaction {
            version: transaction::Version::ONE,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(funding.compute_txid(), 0),
                script_sig: Default::default(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![],
        };
        let mut reveal = create_reveal_transaction_stub(vec![1; 126], &OL_STF_CHECKPOINT_TX_TAG);
        reveal.input[0].previous_output = OutPoint::new(commit.compute_txid(), 0);
        for tx in [&funding, &commit] {
            db.broadcast_db()
                .put_tx_entry(tx.compute_txid().to_buf32(), L1TxEntry::from_tx(tx))
                .unwrap();
        }
        assert_eq!(
            publication_for_tx(db.as_ref(), &commit).unwrap().1,
            PublishDecision::Defer
        );
        db.broadcast_db()
            .put_tx_entry(
                reveal.compute_txid().to_buf32(),
                L1TxEntry::from_tx(&reveal),
            )
            .unwrap();

        for (tx, expected) in [
            (&commit, PublishDecision::Defer),
            (&reveal, PublishDecision::Publish),
        ] {
            assert_eq!(publication_for_tx(db.as_ref(), tx).unwrap().1, expected);
        }

        let mut published_commit = L1TxEntry::from_tx(&commit);
        published_commit.status = L1TxStatus::Published;
        db.broadcast_db()
            .put_tx_entry(commit.compute_txid().to_buf32(), published_commit)
            .unwrap();
        let next_commit = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(reveal.compute_txid(), 1),
                script_sig: Default::default(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![],
        };
        db.broadcast_db()
            .put_tx_entry(
                next_commit.compute_txid().to_buf32(),
                L1TxEntry::from_tx(&next_commit),
            )
            .unwrap();
        assert_eq!(
            publication_for_tx(db.as_ref(), &reveal).unwrap().1,
            PublishDecision::Publish
        );
    }

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
