//! Duty executor worker for sequencer.

use std::{collections::HashSet, sync::Arc};

use anyhow::{Result, anyhow};
use ssz::Encode;
use strata_asm_proto_checkpoint_txs::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
use strata_btcio::writer::EnvelopeHandle;
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_consensus_logic::message::ForkChoiceMessage;
use strata_crypto::hash;
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_l1_txfmt::TagData;
use strata_ol_sequencer::{BlockSigningDuty, CheckpointSigningDuty, Duty, TemplateManager};
use strata_primitives::buf::Buf32;
use strata_storage::NodeStorage;
use tokio::{runtime::Handle, select, sync::mpsc, time};
use tracing::{debug, error, info, warn};

use super::helpers::{sign_checkpoint, sign_header};

/// Worker for executing duties for the sequencer.
pub(crate) async fn duty_executor_worker(
    template_manager: Arc<TemplateManager>,
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<strata_consensus_logic::FcmServiceHandle>,
    mut duty_rx: mpsc::Receiver<Duty>,
    handle: Handle,
    sequencer_key: Buf32,
) -> Result<()> {
    let mut seen_duties = HashSet::new();
    let (failed_duties_tx, mut failed_duties_rx) = mpsc::channel::<Buf32>(8);

    loop {
        select! {
            duty = duty_rx.recv() => {
                if let Some(duty) = duty {
                    let duty_id = duty.generate_id();
                    if seen_duties.contains(&duty_id) {
                        debug!(?duty_id, "skipping already seen duty");
                        continue;
                    }
                    seen_duties.insert(duty_id);
                    handle.spawn(handle_duty(
                        template_manager.clone(),
                        envelope_handle.clone(),
                        storage.clone(),
                        fcm_handle.clone(),
                        duty,
                        sequencer_key,
                        failed_duties_tx.clone(),
                    ));
                } else {
                    return Ok(());
                }
            }
            failed_duty = failed_duties_rx.recv() => {
                if let Some(duty_id) = failed_duty {
                    warn!(?duty_id, "removing failed duty");
                    seen_duties.remove(&duty_id);
                }
            }
        }
    }
}

/// Handles a duty for the sequencer.
async fn handle_duty(
    template_manager: Arc<TemplateManager>,
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<strata_consensus_logic::FcmServiceHandle>,
    duty: Duty,
    sequencer_key: Buf32,
    failed_duties_tx: mpsc::Sender<Buf32>,
) {
    let duty_id = duty.generate_id();
    debug!(?duty_id, ?duty, "handle_duty");
    let duty_result = match duty.clone() {
        Duty::SignBlock(duty) => {
            handle_sign_block_duty(
                template_manager,
                storage,
                fcm_handle,
                duty,
                duty_id,
                &sequencer_key,
            )
            .await
        }
        Duty::SignCheckpoint(duty) => {
            handle_sign_checkpoint_duty(envelope_handle, duty, duty_id, &sequencer_key).await
        }
    };

    if let Err(err) = duty_result {
        error!(?duty_id, %err, "duty failed");
        let _ = failed_duties_tx.send(duty_id).await;
    }
}

/// Handles a block signing duty for the sequencer.
async fn handle_sign_block_duty(
    template_manager: Arc<TemplateManager>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<strata_consensus_logic::FcmServiceHandle>,
    duty: BlockSigningDuty,
    duty_id: Buf32,
    sequencer_key: &Buf32,
) -> Result<()> {
    if let Some(wait_duration) = duty.wait_duration() {
        warn!(?duty_id, "got duty too early; sleeping till target time");
        time::sleep(wait_duration).await;
    }

    let signature = sign_header(duty.template.header(), sequencer_key);

    let block = template_manager
        .complete_template(duty.template_id(), signature)
        .await
        .map_err(|e| anyhow!("failed completing template: {e}"))?;

    storage
        .ol_block()
        .put_block_data_async(block.clone())
        .await
        .map_err(|e| anyhow!("failed storing block: {e}"))?;

    let blkid = block.header().compute_blkid();
    let submitted = fcm_handle
        .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(blkid))
        .await;
    if !submitted {
        return Err(anyhow!("failed submitting block to fork choice manager"));
    }

    info!(
        ?duty_id,
        block_id = ?blkid,
        slot = block.header().slot(),
        "block signing complete"
    );

    Ok(())
}

/// Handles a checkpoint signing duty for the sequencer.
async fn handle_sign_checkpoint_duty(
    envelope_handle: Arc<EnvelopeHandle>,
    duty: CheckpointSigningDuty,
    duty_id: Buf32,
    sequencer_key: &Buf32,
) -> Result<()> {
    let sig = sign_checkpoint(duty.checkpoint(), sequencer_key);
    let signed_checkpoint = SignedCheckpointPayload::new(duty.checkpoint().clone(), sig);
    let checkpoint_tag = TagData::new(
        CHECKPOINT_V0_SUBPROTOCOL_ID,
        OL_STF_CHECKPOINT_TX_TYPE,
        vec![],
    )
    .map_err(|e| anyhow!("failed to build checkpoint tag: {e}"))?;

    let payload = L1Payload::new(vec![signed_checkpoint.as_ssz_bytes()], checkpoint_tag);
    let sighash = hash::raw(&signed_checkpoint.inner().as_ssz_bytes());
    let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, payload);

    envelope_handle
        .submit_intent_async(payload_intent)
        .await
        .map_err(|e| anyhow!("failed to submit checkpoint intent: {e}"))?;

    info!(
        ?duty_id,
        epoch = duty.epoch(),
        "checkpoint signing complete"
    );

    Ok(())
}
