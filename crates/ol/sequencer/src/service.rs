//! Sequencer service definition for the `strata-service` framework.
//!
//! After the signer extraction, this service is a pure template-generation
//! worker.  All signing is handled externally by `strata-signer` via RPC.

use std::{marker::PhantomData, sync::Arc, time::Instant};

use async_trait::async_trait;
use metrics::{counter, histogram};
use serde::Serialize;
use strata_db_types::errors::DbError;
use strata_ol_block_assembly::BlockAssemblyError;
use strata_primitives::OLBlockId;
use strata_service::{AsyncService, Response, Service, ServiceState};
use tracing::{debug, error};

use super::input::SequencerEvent;

/// Status exposed by the sequencer service monitor.
#[derive(Clone, Debug, Serialize)]
pub struct SequencerServiceStatus {
    templates_generated: u64,
}

/// Error boundary for infrastructure operations provided by [`SequencerContext`].
#[derive(Debug, thiserror::Error)]
pub enum SequencerContextError {
    #[error("db: {0}")]
    Db(#[from] DbError),

    #[error("template generation failed at tip {tip_blkid}")]
    TemplateGeneration {
        tip_blkid: OLBlockId,
        #[source]
        source: BlockAssemblyError,
    },
}

/// Behavioral runtime abstraction for the sequencer's template generation.
#[async_trait]
pub trait SequencerContext: Send + Sync + 'static {
    async fn generate_template_for_tip(&self) -> Result<Option<OLBlockId>, SequencerContextError>;
}

/// Service state for the sequencer.
pub struct SequencerServiceState<C: SequencerContext> {
    context: Arc<C>,
    last_seen_tip: Option<OLBlockId>,
    templates_generated: u64,
}

impl<C: SequencerContext> SequencerServiceState<C> {
    pub fn new(context: Arc<C>) -> Self {
        Self {
            context,
            last_seen_tip: None,
            templates_generated: 0,
        }
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
            templates_generated: state.templates_generated,
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

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match &input {
            SequencerEvent::GenerationTick => process_generation_tick(state).await,
        }

        Ok(Response::Continue)
    }
}

async fn process_generation_tick<C: SequencerContext>(state: &mut SequencerServiceState<C>) {
    debug!(last_seen_tip = ?state.last_seen_tip, "generation tick fired");

    let generation_started = Instant::now();
    let generation_result = state.context.generate_template_for_tip().await;
    histogram!("strata_sequencer_template_generation_duration_us")
        .record(generation_started.elapsed().as_micros() as f64);

    let generated_tip = match generation_result {
        Ok(tip) => tip,
        Err(err) => {
            error!(%err, "failed to generate template on generation tick");
            return;
        }
    };

    let Some(generated_tip) = generated_tip else {
        debug!("generation tick skipped");
        return;
    };

    let previous_tip = state.last_seen_tip;
    state.last_seen_tip = Some(generated_tip);

    if previous_tip != state.last_seen_tip {
        state.templates_generated += 1;
        counter!("strata_sequencer_templates_generated_total").increment(1);
        debug!(?previous_tip, current_tip = ?state.last_seen_tip, "sequencer tip changed");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use strata_primitives::{Buf32, OLBlockId};

    use super::*;

    #[derive(Debug)]
    struct MockSequencerContext {
        results: Mutex<VecDeque<Result<Option<OLBlockId>, SequencerContextError>>>,
    }

    impl MockSequencerContext {
        fn new(
            results: impl IntoIterator<Item = Result<Option<OLBlockId>, SequencerContextError>>,
        ) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
            }
        }
    }

    #[async_trait]
    impl SequencerContext for MockSequencerContext {
        async fn generate_template_for_tip(
            &self,
        ) -> Result<Option<OLBlockId>, SequencerContextError> {
            self.results
                .lock()
                .expect("mock sequencer context mutex poisoned")
                .pop_front()
                .expect("test should provide a generation result")
        }
    }

    fn block_id(byte: u8) -> OLBlockId {
        OLBlockId::from(Buf32::from([byte; 32]))
    }

    #[tokio::test]
    async fn generation_tick_does_not_count_skipped_second_tick() {
        let tip = block_id(1);
        let context = Arc::new(MockSequencerContext::new([Ok(Some(tip)), Ok(None)]));
        let mut state = SequencerServiceState::new(context);

        SequencerService::<MockSequencerContext>::process_input(
            &mut state,
            SequencerEvent::GenerationTick,
        )
        .await
        .expect("first generation tick should process");
        SequencerService::<MockSequencerContext>::process_input(
            &mut state,
            SequencerEvent::GenerationTick,
        )
        .await
        .expect("second generation tick should process");

        let status = SequencerService::<MockSequencerContext>::get_status(&state);
        assert_eq!(status.templates_generated, 1);
        assert_eq!(state.last_seen_tip, Some(tip));
    }
}
