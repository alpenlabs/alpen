//! RPC server implementation for sequencer.

use std::sync::Arc;

use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use ssz::Encode;
use strata_asm_proto_checkpoint_txs::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
use strata_btcio::writer::EnvelopeHandle;
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_crypto::hash;
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_db_types::types::OLCheckpointStatus;
use strata_identifiers::{Epoch, OLBlockId};
use strata_l1_txfmt::TagData;
use strata_ol_rpc_api::OLSequencerRpcServer;
use strata_ol_rpc_types::RpcDuty;
use strata_ol_sequencer::{BlockCompletionData, TemplateManager, extract_duties};
use strata_primitives::{Buf64, HexBytes64};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::rpc::errors::{db_error, internal_error, not_found_error};

/// Rpc handler for sequencer.
pub(crate) struct OLSeqRpcServer {
    /// Storage backend.
    storage: Arc<NodeStorage>,

    /// Status channel.
    status_channel: Arc<StatusChannel>,

    /// Template manager.
    template_manager: Arc<TemplateManager>,

    /// Envelope handle.
    envelope_handle: Arc<EnvelopeHandle>,
}

impl OLSeqRpcServer {
    /// Creates a new [`OLSeqRpcServer`] instance.
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        template_manager: Arc<TemplateManager>,
        envelope_handle: Arc<EnvelopeHandle>,
    ) -> Self {
        Self {
            storage,
            status_channel,
            template_manager,
            envelope_handle,
        }
    }
}

#[async_trait]
impl OLSequencerRpcServer for OLSeqRpcServer {
    async fn get_sequencer_duties(&self) -> RpcResult<Vec<RpcDuty>> {
        let Some(tip_blkid) = self
            .status_channel
            .get_ol_sync_status()
            .map(|s| *s.tip_blkid())
        else {
            // If there is no tip then there's definitely no checkpoint to sign, so return empty
            // duties.
            return Ok(vec![]);
        };
        let duties = extract_duties(
            self.template_manager.as_ref(),
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
        self.template_manager
            .complete_template(template_id, *completion.signature())
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        Ok(template_id)
    }

    async fn complete_checkpoint_signature(&self, epoch: Epoch, sig: HexBytes64) -> RpcResult<()> {
        let db = self.storage.ol_checkpoint();
        let Some(mut entry) = db.get_checkpoint_async(epoch).await.map_err(db_error)? else {
            return Err(not_found_error(format!(
                "checkpoint {epoch} not found in db"
            )));
        };
        // Assumes that checkpoint db contains only proven checkpoints
        if entry.status == OLCheckpointStatus::Unsigned {
            let signed_checkpoint =
                SignedCheckpointPayload::new(entry.checkpoint.clone(), Buf64(sig.0));
            // TODO: verify sig
            let checkpoint_tag = TagData::new(
                CHECKPOINT_V0_SUBPROTOCOL_ID,
                OL_STF_CHECKPOINT_TX_TYPE,
                vec![],
            )
            .map_err(|e| internal_error(e.to_string()))?;
            let payload = L1Payload::new(vec![signed_checkpoint.as_ssz_bytes()], checkpoint_tag);
            let sighash = hash::raw(&signed_checkpoint.inner().as_ssz_bytes());

            let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, payload);

            let intent_idx = self
                .envelope_handle
                .submit_intent_async_with_idx(payload_intent)
                .await
                .map_err(|e| internal_error(e.to_string()))?
                .ok_or_else(|| internal_error("failed to resolve checkpoint intent index"))?;

            entry.status = OLCheckpointStatus::Signed(intent_idx);
            db.put_checkpoint_async(epoch, entry)
                .await
                .map_err(db_error)?;
        }
        // If already signed, then fine, return
        Ok(())
    }
}
