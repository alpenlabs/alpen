use std::marker::PhantomData;

use serde::Serialize;
use strata_service::{AsyncService, Response, Service, TickMsg};
use tracing::*;

use crate::broadcaster::{
    input::BroadcasterInputMessage, io::BroadcasterIoContext, state::BroadcasterServiceState,
};

/// Broadcaster service status exposed via monitor.
#[derive(Clone, Debug, Serialize)]
pub struct BroadcasterStatus {
    /// Number of currently tracked entries that still need processing/finalization checks.
    pub unfinalized_count: usize,
    /// Next broadcast DB index expected to be discovered by a scan/update pass.
    pub next_idx: u64,
}

/// Broadcaster service implementation.
#[derive(Debug)]
pub(crate) struct BroadcasterService<T>(PhantomData<T>);

impl<C> Service for BroadcasterService<C>
where
    C: BroadcasterIoContext,
{
    type State = BroadcasterServiceState<C>;
    type Msg = TickMsg<BroadcasterInputMessage>;
    type Status = BroadcasterStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        BroadcasterStatus {
            unfinalized_count: state.inner.unfinalized_entries.len(),
            next_idx: state.inner.next_idx,
        }
    }
}

impl<C> AsyncService for BroadcasterService<C>
where
    C: BroadcasterIoContext,
{
    /// No-op launch hook; initialization is performed in state construction.
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    /// Delegates one service input message to broadcaster state processing.
    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        if !state.retry.is_ready() {
            // Notifications follow DB writes; recovery reloads their persisted state.
            return Ok(Response::Continue);
        }

        match state.process_input(input).await {
            Ok(()) => {
                if state.retry.reset() {
                    info!("broadcaster dependencies recovered");
                }
            }
            Err(error) if error.is_retryable() => {
                if state.retry.record_failure() {
                    warn!(%error, "broadcaster dependency unavailable; retrying with backoff");
                }
            }
            Err(error) => {
                error!(%error, "broadcaster service exiting");
                return Err(error.into());
            }
        }

        Ok(Response::Continue)
    }
}
