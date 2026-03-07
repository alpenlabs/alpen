use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::{Result, anyhow};
use serde::Serialize;
use ssz::Encode;
use strata_asm_txs_checkpoint::OL_STF_CHECKPOINT_TX_TAG;
use strata_btcio::writer::EnvelopeHandle;
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_consensus_logic::{FcmServiceHandle, message::ForkChoiceMessage};
use strata_crypto::hash;
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_db_types::types::OLCheckpointStatus;
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_sequencer::{
    BlockCompletionData, BlockSigningDuty, CheckpointSigningDuty, Duty, extract_duties,
};
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::{sync::mpsc, time};
use tracing::{debug, error, info, warn};

use crate::sequencer::{
    helpers::{sign_checkpoint, sign_header},
    input::SequencerEvent,
};

/// Status exposed by the sequencer service monitor.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SequencerServiceStatus {
    pub(crate) duties_dispatched: u64,
    pub(crate) active_duties: u32,
    pub(crate) failed_duty_count: u32,
}

/// Service state for the sequencer.
pub(crate) struct SequencerServiceState {
    pub(crate) blockasm_handle: Arc<BlockasmHandle>,
    pub(crate) envelope_handle: Arc<EnvelopeHandle>,
    pub(crate) storage: Arc<NodeStorage>,
    pub(crate) fcm_handle: Arc<FcmServiceHandle>,
    pub(crate) status_channel: Arc<StatusChannel>,
    pub(crate) sequencer_key: Buf32,
    pub(crate) seen_duties: HashSet<Buf32>,
    pub(crate) active_duties: Arc<AtomicU32>,
    pub(crate) failed_duty_count: Arc<AtomicU32>,
    pub(crate) failed_duties_tx: mpsc::Sender<Buf32>,
    pub(crate) failed_duties_rx: mpsc::Receiver<Buf32>,
    pub(crate) duties_dispatched: u64,
}

impl SequencerServiceState {
    fn duty_context(&self) -> DutyContext {
        DutyContext {
            blockasm_handle: self.blockasm_handle.clone(),
            envelope_handle: self.envelope_handle.clone(),
            storage: self.storage.clone(),
            fcm_handle: self.fcm_handle.clone(),
            sequencer_key: self.sequencer_key,
            active_duties: self.active_duties.clone(),
            failed_duty_count: self.failed_duty_count.clone(),
            failed_duties_tx: self.failed_duties_tx.clone(),
        }
    }
}

impl ServiceState for SequencerServiceState {
    fn name(&self) -> &str {
        "ol_sequencer"
    }
}

/// Context cloned into spawned duty tasks.
#[derive(Clone)]
struct DutyContext {
    blockasm_handle: Arc<BlockasmHandle>,
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<FcmServiceHandle>,
    sequencer_key: Buf32,
    active_duties: Arc<AtomicU32>,
    failed_duty_count: Arc<AtomicU32>,
    failed_duties_tx: mpsc::Sender<Buf32>,
}

/// Async service implementation for the in-node sequencer.
#[derive(Clone, Debug)]
pub(crate) struct SequencerService;

impl Service for SequencerService {
    type State = SequencerServiceState;
    type Msg = SequencerEvent;
    type Status = SequencerServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        SequencerServiceStatus {
            duties_dispatched: state.duties_dispatched,
            active_duties: state.active_duties.load(Ordering::Relaxed),
            failed_duty_count: state.failed_duty_count.load(Ordering::Relaxed),
        }
    }
}

impl AsyncService for SequencerService {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn before_shutdown(
        state: &mut Self::State,
        err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        let active = state.active_duties.load(Ordering::Relaxed);
        if let Some(err) = err {
            warn!(%active, %err, "sequencer service shutting down with error");
        } else {
            info!(%active, "sequencer service shutting down");
        }
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            SequencerEvent::Tick => process_tick(state).await,
        }

        Ok(Response::Continue)
    }
}

#[tracing::instrument(skip_all, fields(component = "sequencer_service"))]
async fn process_tick(state: &mut SequencerServiceState) {
    while let Ok(duty_id) = state.failed_duties_rx.try_recv() {
        warn!(?duty_id, "removing failed duty");
        state.seen_duties.remove(&duty_id);
    }

    let Some(tip_blkid) = get_tip_blkid(&state.storage, &state.status_channel).await else {
        return;
    };

    let duties = match extract_duties(
        state.blockasm_handle.as_ref(),
        tip_blkid,
        state.storage.as_ref(),
    )
    .await
    {
        Ok(duties) => duties,
        Err(err) => {
            error!(%err, "failed to extract duties");
            return;
        }
    };

    if duties.is_empty() {
        return;
    }

    let duties_display: Vec<String> = duties.iter().map(ToString::to_string).collect();
    debug!(duties = ?duties_display, "got some sequencer duties");

    for duty in duties {
        let duty_id = duty.generate_id();
        if state.seen_duties.contains(&duty_id) {
            debug!(?duty_id, "skipping already seen duty");
            continue;
        }

        state.seen_duties.insert(duty_id);
        state.duties_dispatched += 1;

        let ctx = state.duty_context();
        ctx.active_duties.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            if let Err(err) = handle_duty(&ctx, duty).await {
                error!(?duty_id, %err, "duty failed");
                ctx.failed_duty_count.fetch_add(1, Ordering::Relaxed);
                let _ = ctx.failed_duties_tx.send(duty_id).await;
            }

            ctx.active_duties.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

async fn get_tip_blkid(
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> Option<strata_primitives::OLBlockId> {
    match status_channel.get_ol_sync_status().map(|s| *s.tip_blkid()) {
        Some(tip) => Some(tip),
        None => match storage.ol_block().get_canonical_tip_async().await {
            Ok(Some(commitment)) => Some(*commitment.blkid()),
            Ok(None) => {
                warn!("canonical tip not found yet");
                None
            }
            Err(err) => {
                error!(%err, "failed to load canonical tip");
                None
            }
        },
    }
}

async fn handle_duty(ctx: &DutyContext, duty: Duty) -> Result<()> {
    let duty_id = duty.generate_id();
    debug!(?duty_id, ?duty, "handle_duty");

    match duty {
        Duty::SignBlock(duty) => {
            handle_sign_block_duty(
                ctx.blockasm_handle.clone(),
                ctx.storage.clone(),
                ctx.fcm_handle.clone(),
                duty,
                duty_id,
                &ctx.sequencer_key,
            )
            .await
        }
        Duty::SignCheckpoint(duty) => {
            handle_sign_checkpoint_duty(
                ctx.envelope_handle.clone(),
                ctx.storage.clone(),
                duty,
                duty_id,
                &ctx.sequencer_key,
            )
            .await
        }
    }
}

async fn handle_sign_block_duty(
    blockasm_handle: Arc<BlockasmHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<FcmServiceHandle>,
    duty: BlockSigningDuty,
    duty_id: Buf32,
    sequencer_key: &Buf32,
) -> Result<()> {
    if let Some(wait_duration) = duty.wait_duration() {
        warn!(?duty_id, "got duty too early; sleeping till target time");
        time::sleep(wait_duration).await;
    }

    let signature = sign_header(duty.template.header(), sequencer_key);
    let completion = BlockCompletionData::from_signature(signature);

    let block = blockasm_handle
        .complete_block_template(duty.template_id(), completion)
        .await
        .map_err(|e| anyhow!("failed completing template: {e}"))?;

    storage
        .ol_block()
        .put_block_data_async(block.clone())
        .await
        .map_err(|e| anyhow!("failed storing block: {e}"))?;

    let blkid = block.header().compute_blkid();
    if !fcm_handle
        .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(blkid))
        .await
    {
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

async fn handle_sign_checkpoint_duty(
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    duty: CheckpointSigningDuty,
    duty_id: Buf32,
    sequencer_key: &Buf32,
) -> Result<()> {
    let epoch = duty.epoch();
    let checkpoint_db = storage.ol_checkpoint();
    let Some(mut entry) = checkpoint_db
        .get_checkpoint_async(epoch)
        .await
        .map_err(|e| anyhow!("failed loading checkpoint entry: {e}"))?
    else {
        return Err(anyhow!("missing checkpoint entry for epoch {epoch}"));
    };

    if entry.status != OLCheckpointStatus::Unsigned {
        debug!(?duty_id, %epoch, "checkpoint already signed, skipping");
        return Ok(());
    }

    let sig = sign_checkpoint(duty.checkpoint(), sequencer_key);
    let signed_checkpoint = SignedCheckpointPayload::new(duty.checkpoint().clone(), sig);

    let payload = L1Payload::new(
        vec![signed_checkpoint.as_ssz_bytes()],
        OL_STF_CHECKPOINT_TX_TAG.clone(),
    );
    let sighash = hash::raw(&signed_checkpoint.inner().as_ssz_bytes());
    let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, payload);

    let intent_idx = envelope_handle
        .submit_intent_async_with_idx(payload_intent)
        .await
        .map_err(|e| anyhow!("failed to submit checkpoint intent: {e}"))?
        .ok_or_else(|| anyhow!("failed to resolve checkpoint intent index for epoch {epoch}"))?;

    entry.status = OLCheckpointStatus::Signed(intent_idx);
    checkpoint_db
        .put_checkpoint_async(epoch, entry)
        .await
        .map_err(|e| anyhow!("failed persisting signed checkpoint status: {e}"))?;

    info!(
        ?duty_id,
        %epoch,
        %intent_idx,
        "checkpoint signing complete"
    );

    Ok(())
}
