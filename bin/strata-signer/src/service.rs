//! Signer service definition for the `strata-service` framework.
//!
//! Polls the sequencer node for signing duties on each timer tick,
//! deduplicates them, and spawns async signing tasks.

use std::{collections::HashSet, sync::Arc};

use serde::Serialize;
use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_sequencer::Duty;
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{handlers::handle_duty, input::SignerEvent};

/// Status exposed by the signer service monitor.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SignerServiceStatus {
    pub(crate) duties_processed: u64,
    pub(crate) duties_failed: u64,
}

/// Mutable state held across ticks.
pub(crate) struct SignerServiceState {
    rpc: Arc<ManagedWsClient>,
    sequencer_key: Buf32,
    seen_duties: HashSet<Buf32>,
    failed_tx: mpsc::Sender<Buf32>,
    duties_processed: u64,
    duties_failed: u64,
}

impl SignerServiceState {
    pub(crate) fn new(
        rpc: Arc<ManagedWsClient>,
        sequencer_key: Buf32,
        failed_tx: mpsc::Sender<Buf32>,
    ) -> Self {
        Self {
            rpc,
            sequencer_key,
            seen_duties: HashSet::new(),
            failed_tx,
            duties_processed: 0,
            duties_failed: 0,
        }
    }
}

impl ServiceState for SignerServiceState {
    fn name(&self) -> &str {
        "strata_signer"
    }
}

/// Zero-sized service type.
pub(crate) struct SignerService;

impl Service for SignerService {
    type State = SignerServiceState;
    type Msg = SignerEvent;
    type Status = SignerServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        SignerServiceStatus {
            duties_processed: state.duties_processed,
            duties_failed: state.duties_failed,
        }
    }
}

impl AsyncService for SignerService {
    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match &input {
            SignerEvent::PollTick => process_poll_tick(state).await,
            SignerEvent::DutyFailed(duty_id) => {
                warn!(%duty_id, "removing failed duty for retry");
                state.seen_duties.remove(duty_id);
                state.duties_failed += 1;
            }
        }
        Ok(Response::Continue)
    }
}

/// Fetches duties from the sequencer, deduplicates, and spawns signing tasks.
async fn process_poll_tick(state: &mut SignerServiceState) {
    let rpc_duties = match state.rpc.get_sequencer_duties().await {
        Ok(duties) => duties,
        Err(err) => {
            error!(%err, "failed to fetch sequencer duties");
            return;
        }
    };

    info!(count = rpc_duties.len(), "fetched duties");

    for rpc_duty in rpc_duties {
        let duty: Duty = match rpc_duty.try_into() {
            Ok(d) => d,
            Err(err) => {
                warn!(%err, "failed to convert RpcDuty");
                continue;
            }
        };

        let duty_id = duty.generate_id();
        if state.seen_duties.contains(&duty_id) {
            debug!(%duty_id, "skipping already seen duty");
            continue;
        }
        state.seen_duties.insert(duty_id);
        state.duties_processed += 1;

        tokio::spawn(handle_duty(
            state.rpc.clone(),
            duty,
            state.sequencer_key,
            state.failed_tx.clone(),
        ));
    }
}
