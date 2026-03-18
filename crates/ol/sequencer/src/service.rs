//! Sequencer service definition for the `strata-service` framework.

use std::{
    collections::HashSet,
    marker::PhantomData,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use serde::Serialize;
use ssz::Encode;
use strata_asm_txs_checkpoint::OL_STF_CHECKPOINT_TX_TAG;
use strata_codec::encode_to_vec;
use strata_codec_utils::CodecSsz;
use strata_crypto::hash;
use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
use strata_db_types::{
    errors::DbError,
    types::{OLCheckpointEntry, OLCheckpointStatus},
};
use strata_identifiers::Epoch;
use strata_ol_block_assembly::BlockAssemblyError;
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::{buf::Buf32, OLBlockId};
use strata_service::{AsyncService, Response, Service, ServiceState};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::input::SequencerEvent;
use crate::{
    signing::sign_header, BlockCompletionData, BlockSigningDuty, CheckpointSigningDuty, Duty,
    Error as SequencerError,
};

/// Status exposed by the sequencer service monitor.
#[derive(Clone, Debug, Serialize)]
pub struct SequencerServiceStatus {
    duties_dispatched: u64,
    active_duties: u32,
    failed_duty_count: u32,
}

/// Error boundary for infrastructure operations provided by [`SequencerContext`].
#[derive(Debug, thiserror::Error)]
pub enum SequencerContextError {
    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("duty extraction failed at tip {tip_blkid}")]
    DutyExtraction {
        tip_blkid: OLBlockId,
        #[source]
        source: SequencerError,
    },

    #[error("template completion failed for template {template_id}")]
    TemplateCompletion {
        template_id: OLBlockId,
        #[source]
        source: BlockAssemblyError,
    },

    #[error("template generation failed at tip {tip_blkid}")]
    TemplateGeneration {
        tip_blkid: OLBlockId,
        #[source]
        source: BlockAssemblyError,
    },

    #[error("failed to send block {blkid} to fcm")]
    FcmChannelClosed { blkid: OLBlockId },

    #[error("checkpoint intent submission failed")]
    CheckpointIntentSubmission {
        #[source]
        source: anyhow::Error,
    },
}

/// Error boundary for duty orchestration.
#[derive(Debug, thiserror::Error)]
pub enum SequencerDutyError {
    #[error(transparent)]
    Context(#[from] SequencerContextError),
    #[error("missing checkpoint entry for epoch {epoch}")]
    MissingCheckpoint { epoch: Epoch },
    #[error("failed to resolve checkpoint intent index for epoch {epoch}")]
    ResolveCheckpointIntentIndex { epoch: Epoch },
    #[error("failed to encode checkpoint payload: {0}")]
    CheckpointEncode(String),
}

/// Behavioral runtime abstraction for sequencer dependencies.
#[async_trait]
pub trait SequencerContext: Send + Sync + 'static {
    async fn poll_duties(&self) -> Result<Vec<Duty>, SequencerContextError>;

    async fn generate_template_for_tip(&self) -> Result<Option<OLBlockId>, SequencerContextError>;

    async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        completion: BlockCompletionData,
    ) -> Result<OLBlock, SequencerContextError>;

    async fn store_block(&self, block: OLBlock) -> Result<(), SequencerContextError>;

    async fn submit_chain_tip(&self, blkid: OLBlockId) -> Result<(), SequencerContextError>;

    async fn load_checkpoint(
        &self,
        epoch: Epoch,
    ) -> Result<Option<OLCheckpointEntry>, SequencerContextError>;

    async fn submit_checkpoint_intent(
        &self,
        intent: PayloadIntent,
    ) -> Result<Option<u64>, SequencerContextError>;

    async fn persist_checkpoint(
        &self,
        epoch: Epoch,
        entry: OLCheckpointEntry,
    ) -> Result<(), SequencerContextError>;
}

/// Context cloned into spawned duty tasks.
pub(crate) struct DutyContext<C: SequencerContext> {
    context: Arc<C>,
    sequencer_key: Buf32,
    active_duties: Arc<AtomicU32>,
    failed_duty_count: Arc<AtomicU32>,
    failed_duties_tx: mpsc::Sender<Buf32>,
}

impl<C: SequencerContext> Clone for DutyContext<C> {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            sequencer_key: self.sequencer_key,
            active_duties: self.active_duties.clone(),
            failed_duty_count: self.failed_duty_count.clone(),
            failed_duties_tx: self.failed_duties_tx.clone(),
        }
    }
}

/// RAII guard for active duty counting.
pub(crate) struct ActiveDutyGuard {
    active_duties: Arc<AtomicU32>,
}

impl ActiveDutyGuard {
    fn new(active_duties: Arc<AtomicU32>) -> Self {
        active_duties.fetch_add(1, Ordering::Relaxed);
        Self { active_duties }
    }
}

impl Drop for ActiveDutyGuard {
    fn drop(&mut self) {
        self.active_duties.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Service state for the sequencer.
pub struct SequencerServiceState<C: SequencerContext> {
    context: Arc<C>,
    seen_duties: HashSet<Buf32>,
    duty_context: DutyContext<C>,
    last_seen_tip: Option<OLBlockId>,
    active_duties: Arc<AtomicU32>,
    failed_duty_count: Arc<AtomicU32>,
    failed_duties_rx: mpsc::Receiver<Buf32>,
    duties_dispatched: u64,
}

impl<C: SequencerContext> SequencerServiceState<C> {
    pub fn new(
        context: Arc<C>,
        sequencer_key: Buf32,
        active_duties: Arc<AtomicU32>,
        failed_duty_count: Arc<AtomicU32>,
        failed_duties_tx: mpsc::Sender<Buf32>,
        failed_duties_rx: mpsc::Receiver<Buf32>,
    ) -> Self {
        let duty_context = DutyContext {
            context: context.clone(),
            sequencer_key,
            active_duties: active_duties.clone(),
            failed_duty_count: failed_duty_count.clone(),
            failed_duties_tx: failed_duties_tx.clone(),
        };

        Self {
            context,
            seen_duties: HashSet::new(),
            duty_context,
            last_seen_tip: None,
            active_duties,
            failed_duty_count,
            failed_duties_rx,
            duties_dispatched: 0,
        }
    }

    fn duty_context(&self) -> DutyContext<C> {
        self.duty_context.clone()
    }
}

impl<C: SequencerContext> ServiceState for SequencerServiceState<C> {
    fn name(&self) -> &str {
        "ol_sequencer"
    }
}

/// Async service implementation for the sequencer.
#[derive(Clone, Debug)]
pub struct SequencerService<C: SequencerContext>(PhantomData<C>);

impl<C: SequencerContext> Service for SequencerService<C> {
    type State = SequencerServiceState<C>;
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

impl<C: SequencerContext> AsyncService for SequencerService<C> {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            SequencerEvent::Tick => process_tick(state).await,
            SequencerEvent::GenerationTick => process_generation_tick(state).await,
        }

        Ok(Response::Continue)
    }
}

async fn process_generation_tick<C: SequencerContext>(state: &mut SequencerServiceState<C>) {
    debug!(last_seen_tip = ?state.last_seen_tip, "generation tick fired");

    let generated_tip = match state.context.generate_template_for_tip().await {
        Ok(tip) => tip,
        Err(err) => {
            error!(%err, "failed to generate template on generation tick");
            return;
        }
    };

    if generated_tip.is_none() {
        debug!("generation tick skipped: no canonical tip");
    }

    let previous_tip = state.last_seen_tip;
    state.last_seen_tip = generated_tip;

    if previous_tip != state.last_seen_tip {
        debug!(?previous_tip, current_tip = ?state.last_seen_tip, "sequencer tip changed");
    }
}

async fn process_tick<C: SequencerContext>(state: &mut SequencerServiceState<C>) {
    while let Ok(duty_id) = state.failed_duties_rx.try_recv() {
        warn!(?duty_id, "removing failed duty");
        state.seen_duties.remove(&duty_id);
    }

    let duties = match state.context.poll_duties().await {
        Ok(duties) => duties,
        Err(err) => {
            error!(%err, "failed to poll sequencer duties");
            return;
        }
    };

    if duties.is_empty() {
        return;
    }

    let duties_display: Vec<String> = duties.iter().map(ToString::to_string).collect();
    info!(duties = ?duties_display, "got some sequencer duties");

    for duty in duties {
        let duty_id = duty.generate_id();
        if state.seen_duties.contains(&duty_id) {
            debug!(?duty_id, "skipping already seen duty");
            continue;
        }

        state.seen_duties.insert(duty_id);
        state.duties_dispatched += 1;

        let ctx = state.duty_context();
        tokio::spawn(async move {
            let _active_duty_guard = ActiveDutyGuard::new(ctx.active_duties.clone());
            if let Err(err) = handle_duty(&ctx, duty).await {
                error!(?duty_id, %err, "duty failed");
                ctx.failed_duty_count.fetch_add(1, Ordering::Relaxed);
                if ctx.failed_duties_tx.send(duty_id).await.is_err() {
                    error!(?duty_id, "failed duties channel closed, duty lost");
                }
            }
        });
    }
}

async fn handle_duty<C: SequencerContext>(
    ctx: &DutyContext<C>,
    duty: Duty,
) -> Result<(), SequencerDutyError> {
    let duty_id = duty.generate_id();
    debug!(?duty_id, ?duty, "handle_duty");

    match duty {
        Duty::SignBlock(duty) => {
            handle_sign_block_duty(ctx.context.as_ref(), duty, duty_id, &ctx.sequencer_key).await
        }
        Duty::SignCheckpoint(duty) => {
            handle_checkpoint_duty(ctx.context.as_ref(), duty, duty_id).await
        }
    }
}

async fn handle_sign_block_duty<C: SequencerContext>(
    context: &C,
    duty: BlockSigningDuty,
    duty_id: Buf32,
    sequencer_key: &Buf32,
) -> Result<(), SequencerDutyError> {
    if let Some(wait_duration) = duty.wait_duration() {
        warn!(?duty_id, "got duty too early; sleeping till target time");
        tokio::time::sleep(wait_duration).await;
    }

    let signature = sign_header(duty.template.header(), sequencer_key);
    let completion = BlockCompletionData::from_signature(signature);

    let block = context
        .complete_block_template(duty.template_id(), completion)
        .await?;
    context.store_block(block.clone()).await?;

    let blkid = block.header().compute_blkid();
    context.submit_chain_tip(blkid).await?;

    info!(
        ?duty_id,
        block_id = ?blkid,
        slot = block.header().slot(),
        "block signing complete"
    );

    Ok(())
}

/// Handles a checkpoint duty for the sequencer.
///
/// Encodes the checkpoint payload with [`CodecSsz`] and submits it as an L1 payload
/// intent. The envelope builder will use the sequencer's keypair as the taproot key,
/// so the script-spend signature transitively authenticates the payload (SPS-51).
async fn handle_checkpoint_duty<C: SequencerContext>(
    context: &C,
    duty: CheckpointSigningDuty,
    duty_id: Buf32,
) -> Result<(), SequencerDutyError> {
    let epoch = duty.epoch();
    let Some(mut entry) = context.load_checkpoint(epoch).await? else {
        return Err(SequencerDutyError::MissingCheckpoint { epoch });
    };

    if entry.status != OLCheckpointStatus::Unsigned {
        debug!(?duty_id, %epoch, "checkpoint already signed, skipping");
        return Ok(());
    }

    let checkpoint = duty.checkpoint();
    let codec_payload = CodecSsz::new(checkpoint.clone());
    let encoded = encode_to_vec(&codec_payload)
        .map_err(|e| SequencerDutyError::CheckpointEncode(e.to_string()))?;
    let checkpoint_tip = checkpoint.new_tip();

    let payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone());
    let sighash = hash::raw(&checkpoint.as_ssz_bytes());
    let payload_intent = PayloadIntent::new(PayloadDest::L1, sighash, payload);

    let intent_idx = context
        .submit_checkpoint_intent(payload_intent)
        .await?
        .ok_or(SequencerDutyError::ResolveCheckpointIntentIndex { epoch })?;

    entry.status = OLCheckpointStatus::Signed(intent_idx);
    context.persist_checkpoint(epoch, entry).await?;

    info!(
        ?duty_id,
        %epoch,
        l1_height = checkpoint_tip.l1_height(),
        l2_commitment = %checkpoint_tip.l2_commitment(),
        %sighash,
        %intent_idx,
        "checkpoint duty complete"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{
            atomic::{AtomicU32, Ordering},
            Arc,
        },
    };

    use proptest::prelude::*;
    use strata_checkpoint_types_ssz::test_utils::create_test_checkpoint_payload;
    use strata_csm_types::PayloadIntent;
    use strata_identifiers::test_utils::buf32_strategy;
    use strata_ol_block_assembly::FullBlockTemplate;
    use strata_ol_chain_types_new::test_utils::{ol_block_body_strategy, ol_block_header_strategy};
    use tokio::{
        runtime::Runtime,
        sync::{mpsc, Mutex},
    };

    use super::*;
    use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty};

    /// Mock context that returns configurable duties and fails block completion.
    struct MockContext {
        duties: Mutex<Vec<Duty>>,
    }

    impl MockContext {
        fn new(duties: Vec<Duty>) -> Self {
            Self {
                duties: Mutex::new(duties),
            }
        }
    }

    #[async_trait]
    impl SequencerContext for MockContext {
        async fn poll_duties(&self) -> Result<Vec<Duty>, SequencerContextError> {
            Ok(self.duties.lock().await.clone())
        }

        async fn generate_template_for_tip(
            &self,
        ) -> Result<Option<OLBlockId>, SequencerContextError> {
            Ok(None)
        }

        async fn complete_block_template(
            &self,
            template_id: OLBlockId,
            _completion: BlockCompletionData,
        ) -> Result<OLBlock, SequencerContextError> {
            Err(SequencerContextError::TemplateCompletion {
                template_id,
                source: BlockAssemblyError::UnknownTemplateId(template_id),
            })
        }

        async fn store_block(&self, _block: OLBlock) -> Result<(), SequencerContextError> {
            Ok(())
        }

        async fn submit_chain_tip(&self, _blkid: OLBlockId) -> Result<(), SequencerContextError> {
            Ok(())
        }

        async fn load_checkpoint(
            &self,
            _epoch: Epoch,
        ) -> Result<Option<OLCheckpointEntry>, SequencerContextError> {
            Ok(None)
        }

        async fn submit_checkpoint_intent(
            &self,
            _intent: PayloadIntent,
        ) -> Result<Option<u64>, SequencerContextError> {
            Ok(None)
        }

        async fn persist_checkpoint(
            &self,
            _epoch: Epoch,
            _entry: OLCheckpointEntry,
        ) -> Result<(), SequencerContextError> {
            Ok(())
        }
    }

    fn create_test_state(
        context: Arc<MockContext>,
        sequencer_key: Buf32,
    ) -> SequencerServiceState<MockContext> {
        let active_duties = Arc::new(AtomicU32::new(0));
        let failed_duty_count = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel(8);
        SequencerServiceState::new(
            context,
            sequencer_key,
            active_duties,
            failed_duty_count,
            tx,
            rx,
        )
    }

    fn block_duty_strategy() -> impl Strategy<Value = Duty> {
        (ol_block_header_strategy(), ol_block_body_strategy()).prop_map(|(header, body)| {
            Duty::SignBlock(BlockSigningDuty::new(FullBlockTemplate::new(header, body)))
        })
    }

    fn checkpoint_duty_strategy() -> impl Strategy<Value = Duty> {
        any::<u32>().prop_map(|epoch| {
            Duty::SignCheckpoint(CheckpointSigningDuty::new(create_test_checkpoint_payload(
                epoch,
            )))
        })
    }

    #[test]
    fn active_duty_guard_decrements_on_panic() {
        let active = Arc::new(AtomicU32::new(0));
        let active_for_panic = active.clone();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = ActiveDutyGuard::new(active_for_panic);
            panic!("simulated panic while duty is active");
        }));

        assert!(result.is_err());
        assert_eq!(active.load(Ordering::Relaxed), 0);
    }

    proptest! {
        #[test]
        fn process_tick_deduplicates_duties(
            duty in block_duty_strategy(),
            key in buf32_strategy(),
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let context = Arc::new(MockContext::new(vec![duty]));
                let mut state = create_test_state(context, key);

                process_tick(&mut state).await;
                prop_assert_eq!(state.duties_dispatched, 1);

                // Same duty again: should be skipped.
                process_tick(&mut state).await;
                prop_assert_eq!(state.duties_dispatched, 1);

                Ok(())
            })?;
        }

        #[test]
        fn failed_duty_is_requeued(
            duty in block_duty_strategy(),
            key in buf32_strategy(),
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let duty_id = duty.generate_id();
                let context = Arc::new(MockContext::new(vec![duty]));
                let mut state = create_test_state(context, key);

                process_tick(&mut state).await;
                prop_assert_eq!(state.duties_dispatched, 1);
                prop_assert!(state.seen_duties.contains(&duty_id));

                // Simulate a duty failure signal from a spawned task.
                state
                    .duty_context
                    .failed_duties_tx
                    .send(duty_id)
                    .await
                    .expect("failed to enqueue duty failure");

                // Second tick: failed duty removed from seen_duties, re-dispatched.
                process_tick(&mut state).await;
                prop_assert_eq!(state.duties_dispatched, 2);

                Ok(())
            })?;
        }

        #[test]
        fn checkpoint_missing_entry_returns_error(
            duty in checkpoint_duty_strategy(),
            duty_id in buf32_strategy(),
        ) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let context = Arc::new(MockContext::new(vec![]));
                let checkpoint_duty = match duty {
                    Duty::SignCheckpoint(d) => d,
                    _ => unreachable!(),
                };
                let epoch = checkpoint_duty.epoch();

                let result =
                    handle_checkpoint_duty(context.as_ref(), checkpoint_duty, duty_id).await;

                match result {
                    Err(SequencerDutyError::MissingCheckpoint { epoch: e }) => {
                        prop_assert_eq!(e, epoch);
                    }
                    other => prop_assert!(false, "expected MissingCheckpoint, got {:?}", other),
                }

                Ok(())
            })?;
        }
    }
}
