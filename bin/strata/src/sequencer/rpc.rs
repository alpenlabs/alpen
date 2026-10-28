//! RPC server implementation for sequencer.

use std::{future::Future, sync::Arc};

use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use ssz::Encode;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_btcio::writer::{EnvelopeHandle, EnvelopeSigningMode, EnvelopeSigningModeProvider};
use strata_codec::encode_to_vec;
use strata_codec_utils::CodecSsz;
use strata_consensus_logic::{FcmServiceHandle, message::ForkChoiceMessage};
use strata_crypto::{hash, schnorr::verify_schnorr_sig};
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_db_types::{DbResult, common::L1PayloadIntentIndex, l1_writer::L1BundleStatus};
use strata_identifiers::{Epoch, EpochCommitment, OLBlockId};
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_rpc_api::OLSequencerRpcServer;
use strata_ol_rpc_types::RpcDuty;
use strata_ol_sequencer::{BlockCompletionData, CheckpointDutyContext, extract_duties};
use strata_primitives::{
    HexBytes32, HexBytes64,
    buf::{Buf32, Buf64},
};
use strata_status::StatusChannel;
use strata_storage::{NodeStorage, OLCheckpointManager};
use tracing::{info, warn};

use crate::{
    rpc::errors::{db_error, internal_error, not_found_error},
    sequencer::tip::resolve_canonical_tip,
};

/// Rpc handler for sequencer.
pub(crate) struct OLSeqRpcServer {
    /// Storage backend.
    storage: Arc<NodeStorage>,

    /// Status channel for resolving the current canonical tip.
    status_channel: Arc<StatusChannel>,

    checkpoint_ctx: CheckpointRpcContext,

    /// Block assembly handle.
    blockasm_handle: Arc<BlockasmHandle>,

    /// Fork choice manager handle.
    fcm_handle: Arc<FcmServiceHandle>,

    /// Source for the active checkpoint sequencer key.
    signing_mode_provider: Arc<dyn EnvelopeSigningModeProvider>,
}

impl OLSeqRpcServer {
    /// Creates a new [`OLSeqRpcServer`] instance.
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        blockasm_handle: Arc<BlockasmHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        fcm_handle: Arc<FcmServiceHandle>,
        signing_mode_provider: Arc<dyn EnvelopeSigningModeProvider>,
    ) -> Self {
        Self {
            checkpoint_ctx: CheckpointRpcContext::new(&storage, envelope_handle),
            storage,
            status_channel,
            blockasm_handle,
            fcm_handle,
            signing_mode_provider,
        }
    }
}

fn current_pubkey(
    signing_mode_provider: &dyn EnvelopeSigningModeProvider,
) -> anyhow::Result<Option<Buf32>> {
    match signing_mode_provider.signing_mode()? {
        EnvelopeSigningMode::InProcess => Ok(None),
        EnvelopeSigningMode::External { pubkey } => Ok(Some(Buf32(pubkey.serialize()))),
    }
}

#[async_trait]
impl OLSequencerRpcServer for OLSeqRpcServer {
    async fn get_sequencer_duties(&self) -> RpcResult<Vec<RpcDuty>> {
        let Some(tip_commitment) = resolve_canonical_tip(&self.status_channel, &self.storage)
            .await
            .map_err(db_error)?
        else {
            return Ok(vec![]);
        };
        let duties = extract_duties(
            self.blockasm_handle.as_ref(),
            *tip_commitment.blkid(),
            self.storage.as_ref(),
            &self.checkpoint_ctx,
        )
        .await
        .map_err(db_error)?
        .into_iter()
        .map(RpcDuty::from)
        .collect();
        Ok(duties)
    }
    async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        completion: BlockCompletionData,
    ) -> RpcResult<OLBlockId> {
        let block = self
            .blockasm_handle
            .complete_block_template(template_id, completion)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let commitment = self
            .storage
            .ol_block()
            .put_block_data_with_high_watermark_async(block)
            .await
            .map_err(db_error)?;
        let blkid = *commitment.blkid();

        if let Err(error) = self
            .blockasm_handle
            .record_persisted_block(template_id)
            .await
        {
            warn!(
                %blkid,
                ?template_id,
                error = %error,
                "failed to mark block template completed after durable storage commit"
            );
        }

        let submitted = self
            .fcm_handle
            .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(blkid))
            .await;
        if !submitted {
            return Err(internal_error(format!(
                "failed to send block {blkid} to fcm"
            )));
        }

        info!(%blkid, "block template completed, stored, and submitted to fcm");
        Ok(blkid)
    }

    // TODO(STR-2803): move checkpoint intent submission into the node and remove this RPC method.
    async fn complete_checkpoint_signature(&self, epoch: Epoch, _sig: HexBytes64) -> RpcResult<()> {
        // NOTE: The signature parameter is ignored. With the SPS-51 envelope trick,
        // authentication is handled by the envelope's taproot pubkey matching the
        // sequencer predicate. The checkpoint payload is submitted without an
        // explicit signature.
        submit_checkpoint(&self.checkpoint_ctx, epoch).await
    }

    async fn complete_payload_signature(
        &self,
        payload_idx: u64,
        expected_sighash: HexBytes32,
        sig: HexBytes64,
    ) -> RpcResult<()> {
        let l1_writer = self.storage.l1_writer();

        let mut entry = l1_writer
            .get_payload_entry_by_idx_async(payload_idx)
            .await
            .map_err(db_error)?
            .ok_or_else(|| not_found_error(format!("payload entry {payload_idx} not found")))?;

        let stored_sighash = match &entry.status {
            L1BundleStatus::PendingRevealTxSign(h) => *h,
            _ => {
                return Err(internal_error(format!(
                    "payload {payload_idx} is not pending signature (status: {:?})",
                    entry.status
                )));
            }
        };

        if stored_sighash != Buf32(expected_sighash.0) {
            return Err(internal_error(format!(
                "sighash mismatch for payload {payload_idx}: signature is for a stale envelope"
            )));
        }

        let sig = Buf64(sig.0);
        if current_pubkey(self.signing_mode_provider.as_ref())
            .map_err(|e| internal_error(e.to_string()))?
            .is_some_and(|pk| !verify_schnorr_sig(&sig, &stored_sighash, &pk))
        {
            return Err(internal_error(format!(
                "invalid signature for payload {payload_idx}"
            )));
        }

        entry.payload_signature = Some(sig);
        l1_writer
            .put_payload_entry_async(payload_idx, entry)
            .await
            .map_err(db_error)?;

        Ok(())
    }
}

/// Production checkpoint duty and submission context backed by node storage and the
/// L1 envelope handle.
struct CheckpointRpcContext {
    canonical_chain_provider: OLCanonicalChainProviderImpl,
    checkpoint_mgr: Arc<OLCheckpointManager>,
    envelope_handle: Arc<EnvelopeHandle>,
}

impl CheckpointRpcContext {
    fn new(storage: &NodeStorage, envelope_handle: Arc<EnvelopeHandle>) -> Self {
        Self {
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(storage),
            checkpoint_mgr: storage.ol_checkpoint().clone(),
            envelope_handle,
        }
    }
}

impl CheckpointDutyContext for CheckpointRpcContext {
    async fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        self.checkpoint_mgr
            .get_next_unsigned_checkpoint_epoch_async()
            .await
    }
    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        self.canonical_chain_provider
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
    }
    async fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        self.checkpoint_mgr
            .get_checkpoint_payload_entry_async(commitment)
            .await
    }
    async fn has_checkpoint_signing_entry(&self, commitment: EpochCommitment) -> DbResult<bool> {
        Ok(self
            .checkpoint_mgr
            .get_checkpoint_signing_entry_async(commitment)
            .await?
            .is_some())
    }
}

/// Submits checkpoint payloads to L1 and records the submission.
trait CheckpointSubmissionContext: CheckpointDutyContext {
    /// Queues the checkpoint payload intent for L1 posting and returns its index.
    fn submit_checkpoint_intent(
        &self,
        intent: PayloadIntent,
    ) -> impl Future<Output = RpcResult<L1PayloadIntentIndex>> + Send;

    /// Records that `commitment` was submitted under `intent_idx`.
    fn record_checkpoint_submission(
        &self,
        commitment: EpochCommitment,
        intent_idx: L1PayloadIntentIndex,
    ) -> impl Future<Output = DbResult<()>> + Send;
}

impl CheckpointSubmissionContext for CheckpointRpcContext {
    async fn submit_checkpoint_intent(
        &self,
        intent: PayloadIntent,
    ) -> RpcResult<L1PayloadIntentIndex> {
        self.envelope_handle
            .submit_intent_async_with_idx(intent)
            .await
            .map_err(|e| internal_error(e.to_string()))?
            .ok_or_else(|| internal_error("failed to resolve checkpoint intent index"))
    }
    async fn record_checkpoint_submission(
        &self,
        commitment: EpochCommitment,
        intent_idx: L1PayloadIntentIndex,
    ) -> DbResult<()> {
        self.checkpoint_mgr
            .put_checkpoint_signing_entry_async(commitment, intent_idx)
            .await
    }
}

async fn submit_checkpoint(ctx: &impl CheckpointSubmissionContext, epoch: Epoch) -> RpcResult<()> {
    let Some(commitment) = ctx
        .get_canonical_epoch_commitment_at(epoch)
        .await
        .map_err(db_error)?
    else {
        return Err(not_found_error(format!(
            "checkpoint {epoch} not found in db"
        )));
    };
    let Some(checkpoint_payload) = ctx
        .get_checkpoint_payload(commitment)
        .await
        .map_err(db_error)?
    else {
        return Err(not_found_error(format!(
            "checkpoint {epoch} payload not found in db"
        )));
    };
    // Assumes that checkpoint db contains only proven checkpoints
    if !ctx
        .has_checkpoint_signing_entry(commitment)
        .await
        .map_err(db_error)?
    {
        // TODO(STR-3814): move this checkpoint encoding + L1Payload
        // construction out of the RPC handler into a dedicated component so
        // it can be reused and tested independently of the RPC layer.
        let codec_payload = CodecSsz::new(checkpoint_payload.clone());
        let encoded = encode_to_vec(&codec_payload)
            .map_err(|e| internal_error(format!("failed to encode checkpoint: {e}")))?;

        let l1_payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone())
            .map_err(|e| internal_error(format!("failed to build checkpoint payload: {e}")))?;
        let sighash = hash::raw(&checkpoint_payload.as_ssz_bytes());

        let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, l1_payload);

        if ctx
            .get_canonical_epoch_commitment_at(epoch)
            .await
            .map_err(db_error)?
            != Some(commitment)
        {
            return Err(internal_error(format!(
                "checkpoint {epoch} is no longer canonical"
            )));
        }
        let intent_idx = ctx.submit_checkpoint_intent(payload_intent).await?;
        ctx.record_checkpoint_submission(commitment, intent_idx)
            .await
            .map_err(db_error)?;
    } else {
        warn!(%epoch, "received submission for already submitted checkpoint, ignoring.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use strata_db_types::DbError;
    use strata_ol_sequencer::test_utils::CheckpointFixture;
    use strata_storage::test_runtime_handle;

    use super::*;

    struct TestContext {
        fixture: CheckpointFixture,
        intents: Mutex<Vec<PayloadIntent>>,
    }

    impl TestContext {
        fn new() -> Self {
            Self {
                fixture: CheckpointFixture::new(),
                intents: Mutex::new(Vec::new()),
            }
        }

        fn submit(&self, epoch: Epoch) -> RpcResult<()> {
            test_runtime_handle().block_on(submit_checkpoint(self, epoch))
        }

        fn signing_entry(&self, commitment: EpochCommitment) -> Option<L1PayloadIntentIndex> {
            self.fixture
                .storage()
                .ol_checkpoint()
                .get_checkpoint_signing_entry_blocking(commitment)
                .unwrap()
        }
    }

    impl CheckpointDutyContext for TestContext {
        async fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
            self.fixture.get_next_unsigned_checkpoint_epoch().await
        }
        async fn get_canonical_epoch_commitment_at(
            &self,
            epoch: Epoch,
        ) -> DbResult<Option<EpochCommitment>> {
            self.fixture.get_canonical_epoch_commitment_at(epoch).await
        }
        async fn get_checkpoint_payload(
            &self,
            commitment: EpochCommitment,
        ) -> DbResult<Option<CheckpointPayload>> {
            self.fixture.get_checkpoint_payload(commitment).await
        }
        async fn has_checkpoint_signing_entry(
            &self,
            commitment: EpochCommitment,
        ) -> DbResult<bool> {
            self.fixture.has_checkpoint_signing_entry(commitment).await
        }
    }

    impl CheckpointSubmissionContext for TestContext {
        async fn submit_checkpoint_intent(
            &self,
            intent: PayloadIntent,
        ) -> RpcResult<L1PayloadIntentIndex> {
            let mut intents = self.intents.lock().unwrap();
            let index = intents.len() as u64;
            intents.push(intent);
            Ok(index)
        }
        async fn record_checkpoint_submission(
            &self,
            commitment: EpochCommitment,
            index: L1PayloadIntentIndex,
        ) -> DbResult<()> {
            self.fixture
                .storage()
                .ol_checkpoint()
                .put_checkpoint_signing_entry_async(commitment, index)
                .await
        }
    }

    #[test]
    fn test_checkpoint_submission_selects_exact_canonical_payload_and_is_idempotent() {
        let ctx = TestContext::new();
        let fixture = &ctx.fixture;
        let first = fixture.extend(fixture.genesis(), 1);
        fixture.publish(first);
        fixture.store_payload(first);
        ctx.submit(first.epoch()).unwrap();

        // Retain a previously submitted fork payload; it must not suppress the canonical candidate.
        let left = fixture.extend(first, 2);
        let right = fixture.extend(first, 3);
        let (orphan, canonical) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        fixture.publish(orphan);
        fixture.store_payload(orphan);
        ctx.submit(orphan.epoch()).unwrap();
        fixture.publish(canonical);
        let payload = fixture.store_payload(canonical);
        ctx.submit(canonical.epoch()).unwrap();
        assert_eq!(ctx.signing_entry(orphan), Some(1));
        assert_eq!(ctx.signing_entry(canonical), Some(2));
        let intents = ctx.intents.lock().unwrap();
        assert_eq!(intents.len(), 3);
        assert_eq!(intents[2].commitment(), &hash::raw(&payload.as_ssz_bytes()));
        assert_eq!(intents[2].dest(), PayloadDest::L1);
        drop(intents);

        ctx.submit(canonical.epoch()).unwrap();
        assert_eq!(ctx.intents.lock().unwrap().len(), 3);
    }

    #[test]
    fn test_checkpoint_submission_waits_for_canonical_candidate_and_payload() {
        let ctx = TestContext::new();
        let first = ctx.fixture.extend(ctx.fixture.genesis(), 1);
        assert_eq!(
            ctx.submit(first.epoch()).unwrap_err(),
            not_found_error("checkpoint 1 not found in db")
        );
        ctx.fixture.publish(first);
        assert_eq!(
            ctx.submit(first.epoch()).unwrap_err(),
            not_found_error("checkpoint 1 payload not found in db")
        );
        assert_eq!(ctx.signing_entry(first), None);
        assert!(ctx.intents.lock().unwrap().is_empty());
    }

    #[test]
    fn test_checkpoint_submission_rechecks_before_intent_and_retries_replacement() {
        let ctx = TestContext::new();
        let first = ctx.fixture.extend(ctx.fixture.genesis(), 1);
        ctx.fixture.publish(first);
        ctx.fixture.store_payload(first);
        let replacement = ctx.fixture.extend(ctx.fixture.genesis(), 2);
        ctx.fixture.store_payload(replacement);
        ctx.fixture.reorg_after_payload(replacement);

        assert_eq!(
            ctx.submit(first.epoch()).unwrap_err(),
            internal_error("checkpoint 1 is no longer canonical")
        );
        assert!(ctx.intents.lock().unwrap().is_empty());
        assert_eq!(ctx.signing_entry(first), None);
        assert_eq!(ctx.signing_entry(replacement), None);
        ctx.submit(replacement.epoch()).unwrap();
        assert_eq!(ctx.signing_entry(replacement), Some(0));
        assert_eq!(ctx.signing_entry(first), None);
    }

    #[test]
    fn test_checkpoint_submission_does_not_write_on_canonical_read_failure() {
        let ctx = TestContext::new();
        let first = ctx.fixture.extend(ctx.fixture.genesis(), 1);
        ctx.fixture.publish(first);
        ctx.fixture.store_payload(first);
        for successful_reads in [0, 1] {
            ctx.fixture.fail_canonical_read_after(successful_reads);
            assert_eq!(
                ctx.submit(first.epoch()).unwrap_err(),
                db_error(DbError::Busy)
            );
            assert!(ctx.intents.lock().unwrap().is_empty());
            assert_eq!(ctx.signing_entry(first), None);
        }
        // A rollback may remove the epoch instead of replacing its terminal.
        ctx.fixture.reorg_after_payload(ctx.fixture.genesis());
        assert_eq!(
            ctx.submit(first.epoch()).unwrap_err(),
            internal_error("checkpoint 1 is no longer canonical")
        );
        assert!(ctx.intents.lock().unwrap().is_empty());
        assert_eq!(ctx.signing_entry(first), None);
    }
}
