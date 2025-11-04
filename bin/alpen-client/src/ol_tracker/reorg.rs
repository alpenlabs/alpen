use tracing::{debug, error, info, warn};

use super::{
    ctx::OlTrackerCtx,
    state::{build_tracker_state, OlTrackerState},
};
use crate::traits::{
    ol_client::{block_commitments_in_range_checked, chain_status_checked, OlClient},
    storage::Storage,
};

pub(super) async fn handle_reorg<TStorage, TOlClient>(
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) -> eyre::Result<()>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    let genesis_slot = ctx.params.genesis_ol_slot();

    // figure out what is the last common block / fork point
    let ol_status = chain_status_checked(ctx.ol_client.as_ref()).await?;
    let mut fork_point = None;

    // walk back from latest block check
    let mut max_slot = ol_status.latest().slot();
    while max_slot >= genesis_slot {
        let min_slot = max_slot
            .saturating_sub(ctx.reorg_fetch_size)
            .max(ctx.params.genesis_ol_slot());

        warn!(min_slot, max_slot, "checking slot range for fork point");

        let blocks =
            block_commitments_in_range_checked(ctx.ol_client.as_ref(), min_slot, max_slot).await?;

        for block in blocks.iter().rev() {
            if let Some(state) = ctx.storage.ee_account_state(block.blkid().into()).await? {
                // found last common slot
                fork_point = Some(state);
                break;
            }
        }

        max_slot = min_slot - 1;
    }

    let Some(last_common) = fork_point else {
        error!(
            %genesis_slot, "reorg: could not find ol fork block till ol genesis slot",
        );
        eyre::bail!("reorg: could not find ol fork block");
    };

    let slot = last_common.ol_slot();

    warn!(slot, "reorg: found fork point; starting db rollback");

    // revert own db and state to fork point
    ctx.storage.rollback_ee_account_state(slot).await?;

    // update own state
    let next_state = build_tracker_state(last_common, &ol_status, ctx.storage.as_ref()).await?;
    debug!(?next_state, "reorg: next tracker state");
    *state = next_state;

    // send notification updates
    ctx.notify_state_update(state.best_ee_state());
    ctx.notify_consensus_update(state.get_consensus_heads());

    info!("reorg: reorg complete");

    Ok(())
}
