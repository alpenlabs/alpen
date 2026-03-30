//! Bundler service for the btcio L1 writer.
//!
//! Accumulates unbundled intents and flushes them into payload entries on each
//! timer tick.

use std::{mem, sync::Arc};

use serde::Serialize;
use strata_db_types::types::IntentEntry;
use strata_service::{
    AsyncService, AsyncServiceInput, Response, Service, ServiceInput, ServiceState,
};
use strata_storage::ops::writer::EnvelopeDataOps;
use tokio::{sync::mpsc, time::Interval};
use tracing::*;

use crate::writer::bundler::process_unbundled_entries;

#[derive(Debug)]
pub(crate) enum BundlerEvent {
    /// Periodic tick to flush accumulated intents.
    BundleTick,
    /// A new intent received from the envelope handle.
    IntentReceived(IntentEntry),
}

pub(crate) struct BundlerInput {
    interval: Interval,
    intent_rx: mpsc::Receiver<IntentEntry>,
}

impl BundlerInput {
    pub(crate) fn new(interval: Interval, intent_rx: mpsc::Receiver<IntentEntry>) -> Self {
        Self {
            interval,
            intent_rx,
        }
    }
}

impl ServiceInput for BundlerInput {
    type Msg = BundlerEvent;
}

impl AsyncServiceInput for BundlerInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<BundlerEvent>> {
        tokio::select! {
            _ = self.interval.tick() => Ok(Some(BundlerEvent::BundleTick)),
            maybe_intent = self.intent_rx.recv() => {
                match maybe_intent {
                    Some(intent) => Ok(Some(BundlerEvent::IntentReceived(intent))),
                    None => {
                        warn!("Intent receiver closed, stopping bundler task");
                        Ok(None)
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct BundlerStatus {
    pub(crate) pending_intents: usize,
}

pub(crate) struct BundlerState {
    pub(crate) ops: Arc<EnvelopeDataOps>,
    pub(crate) unbundled: Vec<IntentEntry>,
}

impl ServiceState for BundlerState {
    fn name(&self) -> &str {
        "btcio_bundler"
    }
}

pub(crate) struct BundlerService;

impl Service for BundlerService {
    type State = BundlerState;
    type Msg = BundlerEvent;
    type Status = BundlerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        BundlerStatus {
            pending_intents: state.unbundled.len(),
        }
    }
}

impl AsyncService for BundlerService {
    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match &input {
            BundlerEvent::IntentReceived(intent) => {
                state.unbundled.push(intent.clone());
            }
            BundlerEvent::BundleTick => {
                // Pass the accumulated vec by value (same as original bundler_task).
                // process_unbundled_entries returns any entries not yet processed.
                let ops = state.ops.clone();
                let pending = mem::take(&mut state.unbundled);
                state.unbundled = process_unbundled_entries(ops.as_ref(), pending).await?;
            }
        }
        Ok(Response::Continue)
    }
}
