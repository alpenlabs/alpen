use std::{sync::Arc, time::Duration};

use strata_rpc_api::StrataSequencerApiClient;
use strata_sequencer::duty::types::Duty;
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info, warn};

pub(crate) async fn duty_fetcher_worker<R>(
    rpc: Arc<R>,
    duty_tx: mpsc::Sender<Duty>,
    poll_interval: u64,
) -> anyhow::Result<()>
where
    R: StrataSequencerApiClient + Send + Sync + 'static,
{
    let mut interval = interval(Duration::from_millis(poll_interval));
    'top: loop {
        interval.tick().await;
        let duties = match rpc.get_sequencer_duties().await {
            Ok(duties) => duties,
            Err(err) => {
                // log error and try again
                error!("duty_fetcher_worker: failed to get duties: {}", err);
                continue;
            }
        };

        info!(count = %duties.len(), "got new duties");

        for duty in duties {
            if duty_tx.send(duty).await.is_err() {
                warn!("duty_fetcher_worker: rx dropped; exiting");
                break 'top;
            }
        }
    }

    Ok(())
}
