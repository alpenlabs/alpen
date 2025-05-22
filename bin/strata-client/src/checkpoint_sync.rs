use std::sync::Arc;

use bitcoind_async_client::traits::Reader;
use strata_btcio::reader::query::read_checkpoints;
use strata_config::btcio::ReaderConfig;
use strata_consensus_logic::{
    checkpoint_sync::CheckpointSyncManager, checkpoint_verification::verify_checkpoint,
};
use strata_primitives::params::Params;
use strata_state::{
    batch::CheckpointCommitment, da::ChainstateDAScheme, sync_event::EventSubmitter,
    traits::ChainstateDA,
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tokio::time::Duration;
use tracing::{info, warn};

pub async fn checkpoint_sync_task<E: EventSubmitter>(
    client: Arc<impl Reader>,
    storage: Arc<NodeStorage>,
    config: Arc<ReaderConfig>,
    params: Arc<Params>,
    status_channel: StatusChannel,
    event_submitter: Arc<E>,
) -> anyhow::Result<()> {
    // TODO: is there a way for event_receiver to be structured? we might need to wait for CSM to
    // complete block processing before moving on to fetching previous checkpoint?
    let poll_dur = Duration::from_millis(config.client_poll_dur_ms as u64);
    let mut csync_manager = CheckpointSyncManager::new(storage.clone());

    loop {
        while let Some(sc) = read_checkpoints(
            client.clone(),
            storage.clone(),
            config.clone(),
            params.clone(),
            status_channel.clone(),
            event_submitter.as_ref(),
        )
        .await?
        {
            // verify checkpoint proof and commitments
            // (signature is already verified by the reader)
            info!(
                "checkpoint transaction detected, epoch: {}",
                sc.checkpoint().batch_info().epoch()
            );

            // get previous checkpoint from database
            let ckpt_db = storage.checkpoint();
            let prev_checkpoint = if let Some(idx) = ckpt_db.get_last_checkpoint().await? {
                ckpt_db.get_checkpoint(idx).await?
            } else {
                None
            };

            let prev_commitment = prev_checkpoint.map(|ckpt| {
                return CheckpointCommitment::new(
                    ckpt.checkpoint.batch_info().clone(),
                    ckpt.checkpoint.batch_transition().clone(),
                );
            });
            if let Err(err) =
                verify_checkpoint(sc.checkpoint(), prev_commitment.as_ref(), &params.rollup)
            {
                warn!("failed checkpoint verification: {}", err);
                continue;
            }

            // extract chainstate update structure using DA scheme
            if let Ok(chainstate_update) =
                ChainstateDAScheme::chainstate_update_from_bytes(sc.checkpoint().sidecar().bytes())
            {
                // apply chainstate update
                let chainstate_result = csync_manager
                    .apply_chainstate_update(chainstate_update)
                    .await;

                // store chainstate result to database
                if let Ok(cs) = chainstate_result {
                    info!("storing updated chainstate, slot: {}", cs.chain_tip_slot());

                    if let Err(err) = csync_manager.store_chainstate(cs).await {
                        warn!("failed to store chainstate, aborting sync: {}", err);
                        continue;
                    }

                    // TODO: use status channel to send chainstate update status to listeners?
                }
            }
        }

        tokio::time::sleep(poll_dur).await;
    }
}
