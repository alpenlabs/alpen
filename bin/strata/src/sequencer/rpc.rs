//! RPC server implementation for sequencer.

use std::sync::Arc;

use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use ssz::Encode;
use strata_asm_txs_checkpoint::OL_STF_CHECKPOINT_TX_TAG;
use strata_btcio::writer::EnvelopeHandle;
use strata_codec::encode_to_vec;
use strata_codec_utils::CodecSsz;
use strata_consensus_logic::{FcmServiceHandle, message::ForkChoiceMessage};
use strata_crypto::hash;
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_db_types::types::L1BundleStatus;
use strata_identifiers::{Epoch, OLBlockId};
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_rpc_api::OLSequencerRpcServer;
use strata_ol_rpc_types::RpcDuty;
use strata_ol_sequencer::{BlockCompletionData, extract_duties};
use strata_primitives::{
    HexBytes32, HexBytes64,
    buf::{Buf32, Buf64},
};
use strata_storage::NodeStorage;
use tracing::{info, warn};

use crate::rpc::errors::{db_error, internal_error, not_found_error};

/// Rpc handler for sequencer.
pub(crate) struct OLSeqRpcServer {
    /// Storage backend.
    storage: Arc<NodeStorage>,

    /// Block assembly handle.
    blockasm_handle: Arc<BlockasmHandle>,

    /// Envelope handle.
    envelope_handle: Arc<EnvelopeHandle>,

    /// Fork choice manager handle.
    fcm_handle: Arc<FcmServiceHandle>,
}

impl OLSeqRpcServer {
    /// Creates a new [`OLSeqRpcServer`] instance.
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        blockasm_handle: Arc<BlockasmHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        fcm_handle: Arc<FcmServiceHandle>,
    ) -> Self {
        Self {
            storage,
            blockasm_handle,
            envelope_handle,
            fcm_handle,
        }
    }
}

#[async_trait]
impl OLSequencerRpcServer for OLSeqRpcServer {
    async fn get_sequencer_duties(&self) -> RpcResult<Vec<RpcDuty>> {
        let Some(tip_blkid) = self
            .storage
            .ol_block()
            .get_canonical_tip_async()
            .await
            .map_err(db_error)?
            .map(|c| *c.blkid())
        else {
            return Ok(vec![]);
        };
        let duties = extract_duties(
            self.blockasm_handle.as_ref(),
            tip_blkid,
            self.storage.as_ref(),
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

        let blkid = block.header().compute_blkid();

        self.storage
            .ol_block()
            .put_block_data_async(block)
            .await
            .map_err(db_error)?;

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
        let db = self.storage.ol_checkpoint();
        let Some(commitment) = db
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
            .map_err(db_error)?
        else {
            return Err(not_found_error(format!(
                "checkpoint {epoch} not found in db"
            )));
        };
        let Some(checkpoint_payload) = db
            .get_checkpoint_payload_entry_async(commitment)
            .await
            .map_err(db_error)?
        else {
            return Err(not_found_error(format!(
                "checkpoint {epoch} payload not found in db"
            )));
        };
        // Assumes that checkpoint db contains only proven checkpoints
        if db
            .get_checkpoint_signing_entry_async(commitment)
            .await
            .map_err(db_error)?
            .is_none()
        {
            let codec_payload = CodecSsz::new(checkpoint_payload.clone());
            let encoded = encode_to_vec(&codec_payload)
                .map_err(|e| internal_error(format!("failed to encode checkpoint: {e}")))?;

            let l1_payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone());
            let sighash = hash::raw(&checkpoint_payload.as_ssz_bytes());

            let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, l1_payload);

            let intent_idx = self
                .envelope_handle
                .submit_intent_async_with_idx(payload_intent)
                .await
                .map_err(|e| internal_error(e.to_string()))?
                .ok_or_else(|| internal_error("failed to resolve checkpoint intent index"))?;

            db.put_checkpoint_signing_entry_async(commitment, intent_idx)
                .await
                .map_err(db_error)?;
        } else {
            warn!(%epoch, "received submission for already submitted checkpoint, ignoring.");
        }
        Ok(())
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

        entry.payload_signature = Some(Buf64(sig.0));
        l1_writer
            .put_payload_entry_async(payload_idx, entry)
            .await
            .map_err(db_error)?;

        Ok(())
    }
}
