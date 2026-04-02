//! Signer service definition for the `strata-service` framework.
//!
//! Polls the sequencer node for signing duties on each timer tick,
//! deduplicates them, and spawns async signing tasks.

use std::{collections::HashSet, sync::Arc};

use serde::Serialize;
use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_rpc_types::RpcDuty;
use strata_ol_sequencer::Duty;
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{handlers::handle_duty, helpers::SequencerSk, input::SignerEvent};

/// Status exposed by the signer service monitor.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SignerServiceStatus {
    pub(crate) duties_processed: u64,
    pub(crate) duties_failed: u64,
}

/// Mutable state held across ticks.
pub(crate) struct SignerServiceState {
    rpc: Arc<ManagedWsClient>,
    /// Sequencer secret key. Stored as a [`SequencerSk`] so spawned duty
    /// handlers receive a pointer clone rather than a byte-level copy of key
    /// material.
    sequencer_key: SequencerSk,
    seen_duties: HashSet<Buf32>,
    failed_tx: mpsc::Sender<Buf32>,
    duties_processed: u64,
    duties_failed: u64,
}

impl SignerServiceState {
    pub(crate) fn new(
        rpc: Arc<ManagedWsClient>,
        sequencer_key: SequencerSk,
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

/// Fetches duties from the sequencer and dispatches them.
async fn process_poll_tick(state: &mut SignerServiceState) {
    let rpc_duties = match state.rpc.get_sequencer_duties().await {
        Ok(duties) => duties,
        Err(err) => {
            error!(%err, "failed to fetch sequencer duties");
            return;
        }
    };

    info!(count = rpc_duties.len(), "fetched duties");
    process_duties(state, rpc_duties).await;
}

/// Deduplicates duties and spawns a signing task for each unseen one.
async fn process_duties(state: &mut SignerServiceState, rpc_duties: Vec<RpcDuty>) {
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
            state.sequencer_key.clone(),
            state.failed_tx.clone(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_common::ws_client::{ManagedWsClient, WsClientConfig};
    use strata_crypto::keys::zeroizable::ZeroizedBuf32;
    use strata_ol_rpc_types::{RpcDuty, RpcPayloadSigningDuty};
    use strata_primitives::{HexBytes32, buf::Buf32};
    use tokio::sync::mpsc;

    use super::*;
    use crate::helpers::SequencerSk;

    fn make_state() -> (SignerServiceState, mpsc::Receiver<Buf32>) {
        let rpc = Arc::new(ManagedWsClient::new_with_default_pool(WsClientConfig {
            url: "ws://127.0.0.1:1".to_string(),
        }));
        let sk: SequencerSk = Arc::new(ZeroizedBuf32::new([0u8; 32]));
        let (failed_tx, failed_rx) = mpsc::channel(8);
        (SignerServiceState::new(rpc, sk, failed_tx), failed_rx)
    }

    fn payload_duty(payload_idx: u64, sighash: [u8; 32]) -> RpcDuty {
        RpcDuty::SignPayload(RpcPayloadSigningDuty {
            payload_idx,
            sighash: HexBytes32(sighash),
        })
    }

    #[tokio::test]
    async fn test_same_duty_not_processed_twice() {
        let (mut state, _rx) = make_state();
        let duty = payload_duty(1, [1u8; 32]);

        process_duties(&mut state, vec![duty.clone()]).await;
        assert_eq!(state.duties_processed, 1);
        assert_eq!(state.seen_duties.len(), 1);

        // Same duty on next poll — must be skipped.
        process_duties(&mut state, vec![duty]).await;
        assert_eq!(state.duties_processed, 1);
    }

    #[tokio::test]
    async fn test_different_duties_both_processed() {
        let (mut state, _rx) = make_state();

        process_duties(
            &mut state,
            vec![payload_duty(1, [1u8; 32]), payload_duty(2, [2u8; 32])],
        )
        .await;

        assert_eq!(state.duties_processed, 2);
        assert_eq!(state.seen_duties.len(), 2);
    }

    #[tokio::test]
    async fn test_failed_duty_removed_for_retry() {
        let (mut state, _rx) = make_state();
        let sighash = Buf32([1u8; 32]);

        process_duties(&mut state, vec![payload_duty(1, [1u8; 32])]).await;
        assert!(state.seen_duties.contains(&sighash));
        assert_eq!(state.duties_processed, 1);

        // Signal failure — duty must be evicted from seen set.
        SignerService::process_input(&mut state, SignerEvent::DutyFailed(sighash))
            .await
            .unwrap();
        assert!(!state.seen_duties.contains(&sighash));
        assert_eq!(state.duties_failed, 1);

        // Same duty now re-dispatched.
        process_duties(&mut state, vec![payload_duty(1, [1u8; 32])]).await;
        assert_eq!(state.duties_processed, 2);
    }
}
