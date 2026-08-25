//! Checkpoint-aware policy evaluated at the final Bitcoin publication boundary.

use std::sync::Arc;

use anyhow::Result;
use bitcoin::Transaction;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::TxInputRef;
use strata_asm_proto_checkpoint_txs::{extract_checkpoint_from_envelope, OL_STF_CHECKPOINT_TX_TAG};
use strata_btc_types::TxidExt;
use strata_btcio::broadcaster::{PublishDecision, PublishPolicy};
use strata_btcio::L1TxEntryExt;
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_db_types::l1_broadcast::L1TxStatus;
use strata_identifiers::Epoch;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use tracing::warn;

use super::context::CheckpointPublishContext;

/// Applies checkpoint safety rules immediately before Bitcoin publication.
#[derive(Debug)]
pub struct CheckpointPublishPolicy<C> {
    context: Arc<C>,
    parser: ParseConfig,
}

impl<C> CheckpointPublishPolicy<C> {
    /// Creates a policy for the configured Bitcoin network.
    pub fn new(context: Arc<C>, magic_bytes: MagicBytes) -> Self {
        Self {
            context,
            parser: ParseConfig::new(magic_bytes),
        }
    }
}

impl<C: CheckpointPublishContext> CheckpointPublishPolicy<C> {
    async fn decide_checkpoint(&self, checkpoint: &CheckpointPayload) -> PublishDecision {
        let checkpoint_epoch = checkpoint.new_tip().epoch;
        let safe_epoch = self
            .context
            .get_safe_checkpoint_epoch()
            .await
            .unwrap_or_else(|err| {
                warn!(%err, "checkpoint safe epoch is unavailable");
                None
            });
        let safe_decision = decide_for_checkpoint_epoch(checkpoint_epoch, safe_epoch);
        if matches!(safe_decision, PublishDecision::Abandon) {
            return safe_decision;
        }

        let latest_epoch = match self.context.get_accepted_checkpoint_epoch().await {
            Ok(Some(epoch)) => epoch,
            Ok(None) => return PublishDecision::Defer,
            Err(err) => {
                warn!(%err, "deferring checkpoint while canonical ASM state is unavailable");
                return PublishDecision::Defer;
            }
        };
        if checkpoint_epoch > latest_epoch {
            return PublishDecision::Publish;
        }

        safe_decision
    }
}

#[async_trait::async_trait]
impl<C: CheckpointPublishContext> PublishPolicy for CheckpointPublishPolicy<C> {
    async fn decide(&self, idx: u64, tx: &Transaction) -> PublishDecision {
        match publication_for_tx(self.context.as_ref(), idx, tx, &self.parser) {
            Ok(Publication::Checkpoint(ckpt_pub)) => decide_for_commit_reveal_pair(
                self.decide_checkpoint(&ckpt_pub.checkpoint).await,
                ckpt_pub.member,
                ckpt_pub.commit_status,
                ckpt_pub.reveal_status,
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

pub(super) fn checkpoint_from_payload(
    payload: &L1Payload,
) -> Result<Option<CheckpointPayload>, ()> {
    let tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
    if payload.tag().subproto_id() != tag.subproto_id()
        || payload.tag().tx_type() != tag.tx_type()
        || payload.tag().aux_data() != tag.aux_data()
    {
        return Ok(None);
    }
    let data = payload.data().flatten().copied().collect::<Vec<_>>();
    decode_buf_exact::<CodecSsz<CheckpointPayload>>(&data)
        .map(CodecSsz::into_inner)
        .map(Some)
        .map_err(|_| ())
}

/// Classifies a reveal from its envelope and a commit from its adjacent reveal entry.
///
/// New writer pairs are persisted atomically at consecutive broadcaster indices.
/// Unknown legacy or partial entries are deferred rather than published blindly.
fn publication_for_tx(
    context: &impl CheckpointPublishContext,
    idx: u64,
    tx: &Transaction,
    parser: &ParseConfig,
) -> Result<Publication> {
    let txid = tx.compute_txid().to_buf32();
    let current = context
        .get_broadcast_entry_by_id(txid)?
        .map(|entry| entry.status);
    match classify_tx(tx, parser) {
        Ok(RevealKind::Checkpoint(checkpoint)) => {
            let commit_status = tx
                .input
                .first()
                .filter(|input| input.previous_output.vout == 0)
                .map(|input| {
                    context.get_broadcast_entry_by_id(input.previous_output.txid.to_buf32())
                })
                .transpose()?
                .flatten()
                .map(|entry| entry.status);
            let ckpt_pub =
                CheckpointPublication::new(checkpoint, PairMember::Reveal, commit_status, current);
            return Ok(Publication::Checkpoint(ckpt_pub));
        }
        Ok(RevealKind::Other) => return Ok(Publication::Other),
        Err(()) => {}
    }

    let Some(reveal_idx) = idx.checked_add(1) else {
        return Ok(Publication::Unknown);
    };
    if reveal_idx >= context.get_next_broadcast_idx()? {
        return Ok(Publication::Unknown);
    }
    let Some(reveal_entry) = context.get_broadcast_entry(reveal_idx)? else {
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
    Ok(match classify_tx(&reveal, parser) {
        Ok(RevealKind::Checkpoint(checkpoint)) => {
            Publication::Checkpoint(CheckpointPublication::new(
                checkpoint,
                PairMember::Commit,
                current,
                Some(reveal_entry.status),
            ))
        }
        Ok(RevealKind::Other) => Publication::Other,
        Err(()) => Publication::Unknown,
    })
}

enum Publication {
    Checkpoint(CheckpointPublication),
    Other,
    Unknown,
}

struct CheckpointPublication {
    checkpoint: Box<CheckpointPayload>,
    member: PairMember,
    commit_status: Option<L1TxStatus>,
    reveal_status: Option<L1TxStatus>,
}

impl CheckpointPublication {
    fn new(
        checkpoint: Box<CheckpointPayload>,
        member: PairMember,
        commit_status: Option<L1TxStatus>,
        reveal_status: Option<L1TxStatus>,
    ) -> Self {
        Self {
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

fn classify_tx(tx: &Transaction, parser: &ParseConfig) -> Result<RevealKind, ()> {
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
fn decide_for_checkpoint_epoch(
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
/// Otherwise the commit is decided first and the reveal waits until that commit
/// is observed, so recovery never rejects a reveal merely because its original
/// commit is temporarily absent from the local Bitcoin node.
fn decide_for_commit_reveal_pair(
    decision: PublishDecision,
    member: PairMember,
    commit_status: Option<L1TxStatus>,
    reveal_status: Option<L1TxStatus>,
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
    let sibling_dead = match member {
        PairMember::Commit => reveal_status.as_ref(),
        PairMember::Reveal => commit_status.as_ref(),
    }
    .is_some_and(|status| !status.may_be_live());
    if sibling_dead {
        return match decision {
            PublishDecision::Publish => PublishDecision::Invalidate,
            decision => decision,
        };
    }

    match member {
        PairMember::Commit => decision,
        PairMember::Reveal => PublishDecision::Defer,
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::opcodes::all::OP_RETURN;
    use bitcoin::script::PushBytesBuf;
    use bitcoin::{
        absolute, transaction, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_asm_proto_txs_test_utils::{create_reveal_transaction_stub, TEST_MAGIC_BYTES};
    use strata_btc_types::TxidExt;
    use strata_codec::encode_to_vec;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_store_sled::SledBackend;
    use strata_db_types::backend::DatabaseBackend;
    use strata_db_types::l1_broadcast::{L1BroadcastDatabase, L1TxEntry};
    use strata_identifiers::Buf32;

    use super::*;
    use crate::checkpoint::CheckpointContextResult;

    #[derive(Debug)]
    struct TestPublishContext {
        db: Arc<SledBackend>,
        accepted_epoch: Option<Epoch>,
        safe_epoch: Option<Epoch>,
    }

    impl TestPublishContext {
        fn new(accepted_epoch: Option<Epoch>, safe_epoch: Option<Epoch>) -> Self {
            Self {
                db: get_test_sled_backend(),
                accepted_epoch,
                safe_epoch,
            }
        }
    }

    #[async_trait::async_trait]
    impl CheckpointPublishContext for TestPublishContext {
        async fn get_accepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
            Ok(self.accepted_epoch)
        }

        async fn get_safe_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
            Ok(self.safe_epoch)
        }

        fn get_next_broadcast_idx(&self) -> CheckpointContextResult<u64> {
            Ok(self.db.broadcast_db().get_next_tx_idx()?)
        }

        fn get_broadcast_entry(&self, idx: u64) -> CheckpointContextResult<Option<L1TxEntry>> {
            Ok(self.db.broadcast_db().get_tx_entry(idx)?)
        }

        fn get_broadcast_entry_by_id(
            &self,
            txid: Buf32,
        ) -> CheckpointContextResult<Option<L1TxEntry>> {
            Ok(self.db.broadcast_db().get_tx_entry_by_id(txid)?)
        }
    }

    #[tokio::test]
    async fn policy_uses_accepted_and_safe_epochs_from_context() {
        let checkpoint = create_test_checkpoint_payload(14);
        let previous_epoch = Epoch::from(13u32);
        let checkpoint_epoch = Epoch::from(14u32);
        for (accepted_epoch, safe_epoch, expected) in [
            (None, None, PublishDecision::Defer),
            (None, Some(checkpoint_epoch), PublishDecision::Abandon),
            (Some(previous_epoch), None, PublishDecision::Publish),
            (
                Some(previous_epoch),
                Some(previous_epoch),
                PublishDecision::Publish,
            ),
            (
                Some(previous_epoch),
                Some(checkpoint_epoch),
                PublishDecision::Abandon,
            ),
            (Some(checkpoint_epoch), None, PublishDecision::Defer),
            (
                Some(checkpoint_epoch),
                Some(previous_epoch),
                PublishDecision::Defer,
            ),
            (
                Some(checkpoint_epoch),
                Some(checkpoint_epoch),
                PublishDecision::Abandon,
            ),
        ] {
            let context = Arc::new(TestPublishContext::new(accepted_epoch, safe_epoch));
            let policy = CheckpointPublishPolicy::new(context, TEST_MAGIC_BYTES);
            assert_eq!(policy.decide_checkpoint(&checkpoint).await, expected);
        }
    }

    #[test]
    fn unlinked_transaction_is_quarantined_but_envelope_transaction_is_known() {
        let context = TestPublishContext::new(None, None);
        let unknown = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        let parser = ParseConfig::new(TEST_MAGIC_BYTES);
        assert!(matches!(
            publication_for_tx(&context, 0, &unknown, &parser).unwrap(),
            Publication::Unknown
        ));

        let mut envelope = create_reveal_transaction_stub(vec![1; 126], &OL_STF_CHECKPOINT_TX_TAG);
        envelope.output.clear();
        assert!(matches!(
            publication_for_tx(&context, 0, &envelope, &parser).unwrap(),
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
            publication_for_tx(&context, 0, &chunked_commit, &parser).unwrap(),
            Publication::Other
        ));
    }

    #[test]
    fn ambiguous_pair_recovers_commit_before_reveal() {
        let context = TestPublishContext::new(None, None);
        let db = &context.db;
        let commit = Transaction {
            version: transaction::Version::ONE,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
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
        let mut commit_entry = L1TxEntry::from_tx(&commit);
        commit_entry.status = L1TxStatus::Submitting;
        let mut reveal_entry = L1TxEntry::from_tx(&reveal);
        reveal_entry.status = L1TxStatus::Submitting;
        db.broadcast_db()
            .put_tx_entry_pair(
                (commit.compute_txid().to_buf32(), commit_entry),
                (reveal.compute_txid().to_buf32(), reveal_entry),
            )
            .unwrap();

        assert_eq!(
            publication_decision(&context, 0, &commit, PublishDecision::Publish),
            PublishDecision::Publish,
        );
        assert_eq!(
            publication_decision(&context, 1, &reveal, PublishDecision::Publish),
            PublishDecision::Defer,
        );

        let mut published_commit = L1TxEntry::from_tx(&commit);
        published_commit.status = L1TxStatus::Published;
        db.broadcast_db()
            .put_tx_entry(commit.compute_txid().to_buf32(), published_commit)
            .unwrap();
        assert_eq!(
            publication_decision(&context, 1, &reveal, PublishDecision::Publish),
            PublishDecision::Publish
        );
    }

    fn publication_decision(
        context: &impl CheckpointPublishContext,
        idx: u64,
        tx: &Transaction,
        decision: PublishDecision,
    ) -> PublishDecision {
        match publication_for_tx(context, idx, tx, &ParseConfig::new(TEST_MAGIC_BYTES)).unwrap() {
            Publication::Other => PublishDecision::Publish,
            Publication::Unknown => PublishDecision::Defer,
            Publication::Checkpoint(CheckpointPublication {
                member,
                commit_status,
                reveal_status,
                ..
            }) => decide_for_commit_reveal_pair(decision, member, commit_status, reveal_status),
        }
    }

    #[test]
    fn checkpoint_and_pair_decisions_cover_safe_publication_states() {
        let epoch = Epoch::from(14u32);
        assert_eq!(
            decide_for_checkpoint_epoch(epoch, None),
            PublishDecision::Defer
        );
        assert_eq!(
            decide_for_checkpoint_epoch(epoch, Some(epoch)),
            PublishDecision::Abandon
        );

        for (decision, member, commit, reveal, expected) in [
            (
                PublishDecision::Abandon,
                PairMember::Commit,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::Queued),
                PublishDecision::Abandon,
            ),
            (
                PublishDecision::Abandon,
                PairMember::Reveal,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::Queued),
                PublishDecision::Defer,
            ),
            (
                PublishDecision::Abandon,
                PairMember::Commit,
                Some(L1TxStatus::Queued),
                Some(L1TxStatus::InvalidInputs),
                PublishDecision::Abandon,
            ),
            (
                PublishDecision::Abandon,
                PairMember::Reveal,
                Some(L1TxStatus::Published),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Publish,
            ),
            (
                PublishDecision::Abandon,
                PairMember::Reveal,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::Unpublished),
                PublishDecision::Defer,
            ),
            (
                PublishDecision::Publish,
                PairMember::Commit,
                Some(L1TxStatus::Submitting),
                Some(L1TxStatus::InvalidInputs),
                PublishDecision::Invalidate,
            ),
            (
                PublishDecision::Publish,
                PairMember::Reveal,
                Some(L1TxStatus::InvalidInputs),
                Some(L1TxStatus::Submitting),
                PublishDecision::Invalidate,
            ),
        ] {
            assert_eq!(
                decide_for_commit_reveal_pair(decision, member, commit, reveal),
                expected
            );
        }
    }
}
