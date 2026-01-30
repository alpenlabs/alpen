use std::{sync::Arc, time::Duration};

use strata_rpc_api_new::OLSequencerRpcClient;
use strata_rpc_types_new::RpcOLDuty;
use tokio::{sync::mpsc, time::interval};
use tracing::{error, info, warn};

pub(crate) async fn duty_fetcher_worker<R>(
    rpc: Arc<R>,
    duty_tx: mpsc::Sender<RpcOLDuty>,
    poll_interval: u64,
) -> anyhow::Result<()>
where
    R: OLSequencerRpcClient + Send + Sync + 'static,
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
