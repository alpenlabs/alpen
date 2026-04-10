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
use strata_service::{AsyncService, Response, Service, ServiceState, TickMsg};
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{handlers::handle_duty, helpers::SequencerSk};

/// Message type for the signer service.
///
/// Either a regular poll tick (time to fetch duties) or a duty-resolution
/// notification (success or failure) carrying the duty ID to evict from the
/// in-flight set.
pub(crate) type SignerMsg = TickMsg<DutyResolved>;

/// Signals duty resolution (success or failure) back to the service loop.
#[derive(Debug)]
pub(crate) struct DutyResolved {
    pub(crate) duty_id: Buf32,
    pub(crate) success: bool,
}

/// Status exposed by the signer service monitor.
#[derive(Clone, Debug, Serialize)]
pub struct SignerServiceStatus {
    pub duties_processed: u64,
    pub duties_failed: u64,
}

/// Mutable state held across ticks.
pub(crate) struct SignerServiceState {
    rpc: Arc<ManagedWsClient>,
    /// Sequencer secret key. Stored as a [`SequencerSk`] so spawned duty
    /// handlers receive a pointer clone rather than a byte-level copy of key
    /// material.
    sequencer_key: SequencerSk,
    executor: TaskExecutor,
    in_flight_duties: HashSet<Buf32>,
    resolved_tx: mpsc::Sender<DutyResolved>,
    duties_processed: u64,
    duties_failed: u64,
}

impl SignerServiceState {
    pub(crate) fn new(
        rpc: Arc<ManagedWsClient>,
        sequencer_key: SequencerSk,
        executor: TaskExecutor,
        resolved_tx: mpsc::Sender<DutyResolved>,
    ) -> Self {
        Self {
            rpc,
            sequencer_key,
            executor,
            in_flight_duties: HashSet::new(),
            resolved_tx,
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
    type Msg = SignerMsg;
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
        match input {
            TickMsg::Tick => state.process_poll_tick().await,
            TickMsg::Msg(DutyResolved { duty_id, success }) => {
                state.in_flight_duties.remove(&duty_id);
                if !success {
                    warn!(%duty_id, "duty failed, will retry if requeued");
                    state.duties_failed += 1;
                }
            }
        }
        Ok(Response::Continue)
    }
}

impl SignerServiceState {
    /// Fetches duties from the sequencer and dispatches them.
    async fn process_poll_tick(&mut self) {
        let rpc_duties = match self.rpc.get_sequencer_duties().await {
            Ok(duties) => duties,
            Err(err) => {
                error!(%err, "failed to fetch sequencer duties");
                return;
            }
        };

        info!(count = rpc_duties.len(), "fetched duties");
        self.process_duties(rpc_duties).await;
    }

    /// Deduplicates duties and spawns a signing task for each unseen one.
    async fn process_duties(&mut self, rpc_duties: Vec<RpcDuty>) {
        for rpc_duty in rpc_duties {
            let duty: Duty = match rpc_duty.try_into() {
                Ok(d) => d,
                Err(err) => {
                    warn!(%err, "failed to convert RpcDuty");
                    continue;
                }
            };

            let duty_id = duty.generate_id();
            if self.in_flight_duties.contains(&duty_id) {
                debug!(%duty_id, "skipping already seen duty");
                continue;
            }
            self.in_flight_duties.insert(duty_id);
            self.duties_processed += 1;

            let rpc = self.rpc.clone();
            let sk = self.sequencer_key.clone();
            let resolved_tx = self.resolved_tx.clone();
            self.executor
                .spawn_critical_async("handle_duty", async move {
                    handle_duty(rpc, duty, sk, resolved_tx).await;
                    Ok(())
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_common::ws_client::{ManagedWsClient, WsClientConfig};
    use strata_crypto::keys::zeroizable::ZeroizedBuf32;
    use strata_ol_rpc_types::{RpcDuty, RpcRevealTxSigningDuty};
    use strata_primitives::{buf::Buf32, HexBytes32};
    use strata_tasks::TaskManager;
    use tokio::{runtime::Handle, sync::mpsc};

    use super::*;
    use crate::helpers::SequencerSk;

    fn make_state() -> (SignerServiceState, mpsc::Receiver<DutyResolved>) {
        let rpc = Arc::new(ManagedWsClient::new_with_default_pool(WsClientConfig {
            url: "ws://127.0.0.1:1".to_string(),
        }));
        let sk: SequencerSk = Arc::new(ZeroizedBuf32::new([0u8; 32]));
        let executor = TaskManager::new(Handle::current()).create_executor();
        let (resolved_tx, resolved_rx) = mpsc::channel(8);
        (
            SignerServiceState::new(rpc, sk, executor, resolved_tx),
            resolved_rx,
        )
    }

    fn payload_duty(payload_idx: u64, sighash: [u8; 32]) -> RpcDuty {
        RpcDuty::SignRevealTx(RpcRevealTxSigningDuty {
            payload_idx,
            sighash: HexBytes32(sighash),
        })
    }

    #[tokio::test]
    async fn test_same_duty_not_processed_twice() {
        let (mut state, _rx) = make_state();
        let duty = payload_duty(1, [1u8; 32]);

        state.process_duties(vec![duty.clone()]).await;
        assert_eq!(state.duties_processed, 1);
        assert_eq!(state.in_flight_duties.len(), 1);

        // Same duty on next poll — must be skipped.
        state.process_duties(vec![duty]).await;
        assert_eq!(state.duties_processed, 1);
    }

    #[tokio::test]
    async fn test_different_duties_both_processed() {
        let (mut state, _rx) = make_state();

        state
            .process_duties(vec![payload_duty(1, [1u8; 32]), payload_duty(2, [2u8; 32])])
            .await;

        assert_eq!(state.duties_processed, 2);
        assert_eq!(state.in_flight_duties.len(), 2);
    }

    #[tokio::test]
    async fn test_failed_duty_removed_for_retry() {
        let (mut state, _rx) = make_state();
        let duty_id = Buf32([1u8; 32]);

        state.process_duties(vec![payload_duty(1, [1u8; 32])]).await;
        assert!(state.in_flight_duties.contains(&duty_id));
        assert_eq!(state.duties_processed, 1);

        // Signal failure — duty must be evicted from seen set.
        SignerService::process_input(
            &mut state,
            TickMsg::Msg(DutyResolved {
                duty_id,
                success: false,
            }),
        )
        .await
        .unwrap();
        assert!(!state.in_flight_duties.contains(&duty_id));
        assert_eq!(state.duties_failed, 1);

        // Same duty now re-dispatched.
        state.process_duties(vec![payload_duty(1, [1u8; 32])]).await;
        assert_eq!(state.duties_processed, 2);
    }

    #[tokio::test]
    async fn test_successful_duty_removed_from_seen() {
        let (mut state, _rx) = make_state();
        let duty_id = Buf32([1u8; 32]);

        state.process_duties(vec![payload_duty(1, [1u8; 32])]).await;
        assert!(state.in_flight_duties.contains(&duty_id));

        // Signal success — duty must be evicted so it can be re-processed if it reappears.
        SignerService::process_input(
            &mut state,
            TickMsg::Msg(DutyResolved {
                duty_id,
                success: true,
            }),
        )
        .await
        .unwrap();
        assert!(!state.in_flight_duties.contains(&duty_id));
        assert_eq!(state.duties_failed, 0);
    }
}
