//! Checkpoint-aware policy evaluated at the final Bitcoin publication boundary.

use std::sync::Arc;

use bitcoin::Transaction;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::{SectionStateExt, Subprotocol, TxInputRef};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_asm_proto_checkpoint_txs::{OL_STF_CHECKPOINT_TX_TAG, extract_checkpoint_from_envelope};
use strata_btc_types::TxidExt;
use strata_btcio::broadcaster::{PublishContext, PublishDecision, PublishPolicy};
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_db_types::{
    DbResult,
    backend::DatabaseBackend,
    l1_broadcast::{L1BroadcastDatabase, L1TxStatus},
};
use strata_identifiers::Epoch;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_storage::NodeStorage;
use tracing::warn;

pub(crate) struct CheckpointPublishPolicy {
    storage: Arc<NodeStorage>,
    parser: ParseConfig,
}

impl CheckpointPublishPolicy {
    pub(crate) fn new(storage: Arc<NodeStorage>, magic_bytes: MagicBytes) -> Self {
        Self {
            storage,
            parser: ParseConfig::new(magic_bytes),
        }
    }

    async fn decide_checkpoint(&self, checkpoint: &CheckpointPayload) -> PublishDecision {
        let latest_state = match self.storage.fetch_canonical_asm_state_async().await {
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

        let safe_epoch = self.safe_checkpoint_epoch().await.unwrap_or_else(|err| {
            warn!(%err, "checkpoint safe epoch is unavailable");
            None
        });
        accepted_checkpoint_decision(checkpoint.new_tip().epoch, safe_epoch)
    }

    /// Returns the final epoch from the latest client state when its L1 anchor is canonical.
    async fn safe_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        let Some((block, state)) = self
            .storage
            .client_state()
            .fetch_most_recent_state_async()
            .await?
        else {
            return Ok(None);
        };
        let canonical = self
            .storage
            .l1()
            .get_canonical_blockid_at_height_async(block.height())
            .await?;
        if canonical != Some(*block.blkid()) {
            return Ok(None);
        }
        Ok(state
            .get_declared_final_epoch()
            .map(|commitment| commitment.epoch()))
    }
}

#[async_trait::async_trait]
impl PublishPolicy for CheckpointPublishPolicy {
    async fn decide(&self, idx: u64, tx: &Transaction, context: PublishContext) -> PublishDecision {
        match publication_for_tx(self.storage.db().as_ref(), idx, tx, &self.parser) {
            Ok(Publication::Checkpoint {
                checkpoint,
                member,
                commit_status,
                reveal_status,
            }) => pair_decision(
                self.decide_checkpoint(&checkpoint).await,
                member,
                commit_status,
                reveal_status,
                context,
            ),
            Ok(Publication::Other) => PublishDecision::Publish,
            Ok(Publication::Unknown) => PublishDecision::Defer,
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

/// Classifies a reveal from its envelope and a commit from its adjacent reveal entry.
///
/// New writer pairs are persisted atomically at consecutive broadcaster indices.
/// Unknown legacy or partial entries are deferred rather than published blindly.
fn publication_for_tx(
    db: &impl DatabaseBackend,
    idx: u64,
    tx: &Transaction,
    parser: &ParseConfig,
) -> DbResult<Publication> {
    let broadcast = db.broadcast_db();
    let txid = tx.compute_txid().to_buf32();
    let current = broadcast
        .get_tx_entry_by_id(txid)?
        .map(|entry| entry.status);
    match reveal_kind(tx, parser) {
        Ok(RevealKind::Checkpoint(checkpoint)) => {
            let commit_status = tx
                .input
                .first()
                .filter(|input| input.previous_output.vout == 0)
                .map(|input| broadcast.get_tx_entry_by_id(input.previous_output.txid.to_buf32()))
                .transpose()?
                .flatten()
                .map(|entry| entry.status);
            return Ok(Publication::checkpoint(
                checkpoint,
                PairMember::Reveal,
                commit_status,
                current,
            ));
        }
        Ok(RevealKind::Other) => return Ok(Publication::Other),
        Err(()) => {}
    }

    let Some(reveal_idx) = idx.checked_add(1) else {
        return Ok(Publication::Unknown);
    };
    if reveal_idx >= broadcast.get_next_tx_idx()? {
        return Ok(Publication::Unknown);
    }
    let Some(reveal_entry) = broadcast.get_tx_entry(reveal_idx)? else {
        return Ok(Publication::Unknown);
    };
    let Ok(reveal) = reveal_entry.try_to_tx() else {
        return Ok(Publication::Unknown);
    };
    if !reveal.input.first().is_some_and(|input| {
        input.previous_output.txid.to_buf32() == txid && input.previous_output.vout == 0
    }) {
        return Ok(Publication::Unknown);
    }
    Ok(match reveal_kind(&reveal, parser) {
        Ok(RevealKind::Checkpoint(checkpoint)) => Publication::checkpoint(
            checkpoint,
            PairMember::Commit,
            current,
            Some(reveal_entry.status),
        ),
        Ok(RevealKind::Other) => Publication::Other,
        Err(()) => Publication::Unknown,
    })
}

enum Publication {
    Checkpoint {
        checkpoint: Box<CheckpointPayload>,
        member: PairMember,
        commit_status: Option<L1TxStatus>,
        reveal_status: Option<L1TxStatus>,
    },
    Other,
    Unknown,
}

impl Publication {
    fn checkpoint(
        checkpoint: Box<CheckpointPayload>,
        member: PairMember,
        commit_status: Option<L1TxStatus>,
        reveal_status: Option<L1TxStatus>,
    ) -> Self {
        Self::Checkpoint {
            checkpoint,
            member,
            commit_status,
            reveal_status,
        }
    }
}

enum RevealKind {
    Checkpoint(Box<CheckpointPayload>),
    Other,
}

fn reveal_kind(tx: &Transaction, parser: &ParseConfig) -> Result<RevealKind, ()> {
    let tag = match parser.try_parse_tx(tx) {
        Ok(tag) => tag,
        Err(_) if is_envelope_reveal(tx) => return Ok(RevealKind::Other),
        Err(_) => return Err(()),
    };
    let checkpoint_tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
    if tag.subproto_id() != checkpoint_tag.subproto_id()
        || tag.tx_type() != checkpoint_tag.tx_type()
        || tag.aux_data() != checkpoint_tag.aux_data()
    {
        return Ok(RevealKind::Other);
    }
    extract_checkpoint_from_envelope(&TxInputRef::new(tx, tag))
        .map(|envelope| RevealKind::Checkpoint(Box::new(envelope.payload)))
        .map_err(|_| ())
}

fn is_envelope_reveal(tx: &Transaction) -> bool {
    tx.input
        .first()
        .and_then(|input| input.witness.taproot_leaf_script())
        .is_some_and(|leaf| parse_envelope_payload(&leaf.script.into()).is_ok())
}

/// Abandons reorg-safe checkpoints and defers accepted checkpoints that are not yet safe.
fn accepted_checkpoint_decision(
    checkpoint_epoch: Epoch,
    safe_epoch: Option<Epoch>,
) -> PublishDecision {
    if safe_epoch.is_some_and(|epoch| checkpoint_epoch <= epoch) {
        PublishDecision::Abandon
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
/// If either transaction reached L1, the other is published to finish the pair.
/// A reveal probes a missing or ambiguously submitted commit; a genuinely absent
/// commit fails safely with invalid inputs. Commit recovery stays deferred until
/// that reveal probe has a definitive outcome. Definitely-unsent pairs use the
/// checkpoint decision directly.
fn pair_decision(
    decision: PublishDecision,
    member: PairMember,
    commit_status: Option<L1TxStatus>,
    reveal_status: Option<L1TxStatus>,
    context: PublishContext,
) -> PublishDecision {
    let reached_l1 = commit_status
        .as_ref()
        .is_some_and(L1TxStatus::has_reached_l1)
        || reveal_status
            .as_ref()
            .is_some_and(L1TxStatus::has_reached_l1);
    if reached_l1 {
        return PublishDecision::Publish;
    }
    if matches!(member, PairMember::Commit)
        && reveal_status
            .as_ref()
            .is_some_and(|status| !status.may_be_live())
    {
        return match decision {
            PublishDecision::Publish => PublishDecision::Invalidate,
            decision => decision,
        };
    }

    let reveal_unresolved = reveal_status
        .as_ref()
        .is_some_and(L1TxStatus::submission_pending);
    match member {
        PairMember::Reveal
            if commit_status.is_none()
                || commit_status
                    .as_ref()
                    .is_some_and(L1TxStatus::submission_started)
                || (context == PublishContext::Recovery
                    && reveal_status
                        .as_ref()
                        .is_some_and(L1TxStatus::submission_started)) =>
        {
            PublishDecision::Publish
        }
        PairMember::Commit
            if reveal_status.is_none()
                || (context == PublishContext::Recovery && reveal_unresolved) =>
        {
            PublishDecision::Defer
        }
        _ => decision,
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute,
        opcodes::all::OP_RETURN, script::PushBytesBuf, transaction,
    };
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_asm_proto_txs_test_utils::{TEST_MAGIC_BYTES, create_reveal_transaction_stub};
    use strata_btc_types::TxidExt;
    use strata_codec::encode_to_vec;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{backend::DatabaseBackend, l1_broadcast::L1TxEntry};
    use strata_storage::{create_node_storage, test_runtime_handle};

    use super::*;

    #[tokio::test]
    async fn storage_backed_policy_is_safe_inside_tokio() {
        let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle())
            .expect("test storage");
        let policy = CheckpointPublishPolicy::new(Arc::new(storage), TEST_MAGIC_BYTES);

        assert_eq!(policy.safe_checkpoint_epoch().await.unwrap(), None);
    }

    #[test]
    fn unlinked_transaction_is_quarantined_but_envelope_transaction_is_known() {
        let db = get_test_sled_backend();
        let unknown = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        let parser = ParseConfig::new(TEST_MAGIC_BYTES);
        assert!(matches!(
            publication_for_tx(db.as_ref(), 0, &unknown, &parser).unwrap(),
            Publication::Unknown
        ));

        let mut envelope = create_reveal_transaction_stub(vec![1; 126], &OL_STF_CHECKPOINT_TX_TAG);
        envelope.output.clear();
        assert!(matches!(
            publication_for_tx(db.as_ref(), 0, &envelope, &parser).unwrap(),
            Publication::Other
        ));

        let mut marker = TEST_MAGIC_BYTES.as_bytes().to_vec();
        marker.extend([0, 0, 0, 1]);
        let marker = PushBytesBuf::try_from(marker).unwrap();
        let mut chunked_commit = unknown;
        chunked_commit.output.push(TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::builder()
                .push_opcode(OP_RETURN)
                .push_slice(marker)
                .into_script(),
        });
        assert!(matches!(
            publication_for_tx(db.as_ref(), 0, &chunked_commit, &parser).unwrap(),
            Publication::Other
        ));
    }

    #[test]
    fn unlinked_legacy_pair_is_deferred_until_reveal_is_known() {
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
        let checkpoint = create_test_checkpoint_payload(14);
        let encoded = encode_to_vec(&CodecSsz::new(checkpoint)).unwrap();
        let mut reveal = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
        reveal.input[0].previous_output = OutPoint::new(commit.compute_txid(), 0);
        for tx in [&funding, &commit] {
            db.broadcast_db()
                .put_tx_entry(tx.compute_txid().to_buf32(), L1TxEntry::from_tx(tx))
                .unwrap();
        }
        assert_eq!(
            publication_decision(db.as_ref(), 1, &commit),
            PublishDecision::Defer,
        );
        db.broadcast_db()
            .put_tx_entry(
                reveal.compute_txid().to_buf32(),
                L1TxEntry::from_tx(&reveal),
            )
            .unwrap();

        for (idx, tx, expected) in [
            (1, &commit, PublishDecision::Abandon),
            (2, &reveal, PublishDecision::Abandon),
        ] {
            assert_eq!(publication_decision(db.as_ref(), idx, tx), expected);
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
            publication_decision(db.as_ref(), 2, &reveal),
            PublishDecision::Publish
        );
    }

    fn publication_decision(
        db: &impl DatabaseBackend,
        idx: u64,
        tx: &Transaction,
    ) -> PublishDecision {
        match publication_for_tx(db, idx, tx, &ParseConfig::new(TEST_MAGIC_BYTES)).unwrap() {
            Publication::Checkpoint {
                member,
                commit_status,
                reveal_status,
                ..
            } => pair_decision(
                PublishDecision::Abandon,
                member,
                commit_status,
                reveal_status,
                PublishContext::Initial,
            ),
            Publication::Other => PublishDecision::Publish,
            Publication::Unknown => PublishDecision::Defer,
        }
    }

    #[test]
    fn checkpoint_and_pair_decisions_cover_safe_publication_states() {
        let epoch = Epoch::from(14u32);
        assert_eq!(
            accepted_checkpoint_decision(epoch, None),
            PublishDecision::Defer
        );
        assert_eq!(
            accepted_checkpoint_decision(epoch, Some(epoch)),
            PublishDecision::Abandon
        );

        for (member, commit, reveal, expected) in [
            (
                PairMember::Commit,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::Queued),
                PublishDecision::Abandon,
            ),
            (
                PairMember::Reveal,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::Queued),
                PublishDecision::Abandon,
            ),
            (
                PairMember::Commit,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::InvalidInputs),
                PublishDecision::Abandon,
            ),
            (
                PairMember::Reveal,
                Some(L1TxStatus::Published),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Publish,
            ),
            (
                PairMember::Reveal,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Publish,
            ),
        ] {
            assert_eq!(
                pair_decision(
                    PublishDecision::Abandon,
                    member,
                    commit,
                    reveal,
                    PublishContext::Initial,
                ),
                expected
            );
        }

        assert_eq!(
            pair_decision(
                PublishDecision::Publish,
                PairMember::Commit,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::InvalidInputs),
                PublishContext::Recovery,
            ),
            PublishDecision::Invalidate
        );
        assert_eq!(
            pair_decision(
                PublishDecision::Abandon,
                PairMember::Commit,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::Unpublished),
                PublishContext::Recovery,
            ),
            PublishDecision::Defer
        );
        assert_eq!(
            pair_decision(
                PublishDecision::Abandon,
                PairMember::Reveal,
                Some(L1TxStatus::Unpublished),
                Some(L1TxStatus::Unpublished),
                PublishContext::Recovery,
            ),
            PublishDecision::Publish
        );
        assert_eq!(
            pair_decision(
                PublishDecision::Abandon,
                PairMember::Commit,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::Submitting),
                PublishContext::Recovery,
            ),
            PublishDecision::Defer
        );
        assert_eq!(
            pair_decision(
                PublishDecision::Abandon,
                PairMember::Reveal,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::Submitting),
                PublishContext::Recovery,
            ),
            PublishDecision::Publish
        );
    }
}
