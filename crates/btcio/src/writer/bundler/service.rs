//! Bundler service for the btcio L1 writer.
//!
//! Accumulates unbundled intents and flushes them into payload entries on each
//! timer tick.

use std::{mem, sync::Arc};

use serde::Serialize;
use strata_db_types::types::IntentEntry;
use strata_service::{AsyncService, Response, Service, ServiceState, TickMsg};
use strata_storage::ops::writer::EnvelopeDataOps;

use super::logic::process_unbundled_entries;

#[derive(Clone, Debug, Serialize)]
pub struct BundlerStatus {
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
    type Msg = TickMsg<IntentEntry>;
    type Status = BundlerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        BundlerStatus {
            pending_intents: state.unbundled.len(),
        }
    }
}

impl AsyncService for BundlerService {
    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            TickMsg::Tick => {
                let pending = mem::take(&mut state.unbundled);
                state.unbundled = process_unbundled_entries(state.ops.as_ref(), pending).await?;
            }
            TickMsg::Msg(intent) => {
                state.unbundled.push(intent);
            }
        }
        Ok(Response::Continue)
    }
}
