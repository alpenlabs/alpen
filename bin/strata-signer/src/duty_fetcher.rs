//! Polls the sequencer node for signing duties and forwards them to the executor.

use std::{sync::Arc, time::Duration};

use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_sequencer::Duty;
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info, warn};

/// Polls `get_sequencer_duties()` on a fixed interval and sends converted duties to the channel.
pub(crate) async fn duty_fetcher_worker(
    rpc: Arc<ManagedWsClient>,
    duty_tx: mpsc::Sender<Duty>,
    duty_poll_interval: u64,
) -> anyhow::Result<()> {
    let mut interval = interval(Duration::from_millis(duty_poll_interval));

    'top: loop {
        interval.tick().await;

        let rpc_duties = match rpc.get_sequencer_duties().await {
            Ok(duties) => duties,
            Err(err) => {
                error!(%err, "failed to fetch sequencer duties");
                continue;
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

            if duty_tx.send(duty).await.is_err() {
                warn!("duty receiver dropped; exiting");
                break 'top;
            }
        }
    }

    Ok(())
}
