//! Bundler service for the btcio L1 writer.
//!
//! Accumulates unbundled intents and flushes them into payload entries on each
//! timer tick.

use std::{collections::BTreeSet, marker::PhantomData, sync::Arc};

use serde::Serialize;
use strata_service::{AsyncService, Response, Service, ServiceState, TickMsg};
use tracing::{info, warn};

use super::logic::{process_unbundled_entries, BundlerStorage, PendingIntent};
use crate::retry::{is_retryable_db_error, RetryBackoff};

#[derive(Clone, Debug, Serialize)]
pub struct BundlerStatus {
    pub(crate) pending_intents: usize,
}

pub(crate) struct BundlerState<C> {
    pub(crate) ops: Arc<C>,
    pub(crate) unbundled: BTreeSet<PendingIntent>,
    pub(crate) retry: RetryBackoff,
}

impl<C: BundlerStorage> ServiceState for BundlerState<C> {
    fn name(&self) -> &str {
        "btcio_bundler"
    }
}

pub(crate) struct BundlerService<C>(PhantomData<C>);

impl<C: BundlerStorage> Service for BundlerService<C> {
    type State = BundlerState<C>;
    type Msg = TickMsg<PendingIntent>;
    type Status = BundlerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        BundlerStatus {
            pending_intents: state.unbundled.len(),
        }
    }
}

impl<C: BundlerStorage> AsyncService for BundlerService<C> {
    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            TickMsg::Tick => {
                if state.unbundled.is_empty() || !state.retry.is_ready() {
                    return Ok(Response::Continue);
                }

                match process_unbundled_entries(state.ops.as_ref(), &mut state.unbundled).await {
                    Ok(()) => {
                        if state.retry.reset() {
                            info!("bundler storage recovered");
                        }
                    }
                    Err(error) if is_retryable_db_error(&error) => {
                        if state.retry.record_failure() {
                            warn!(%error, "bundler storage unavailable; retrying with backoff");
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            TickMsg::Msg(intent) => {
                state.unbundled.insert(intent);
            }
        }
        Ok(Response::Continue)
    }
}
