use std::{sync::Arc, time::Duration};

use strata_ee_acct_runtime::apply_update_operation_unconditionally;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use strata_snark_acct_types::UpdateOperationUnconditionalData;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::{
    ol_tracker::OlTrackerState,
    traits::{ol_client::OlClient, storage::Storage},
};

/// Default number of Ol blocks to process in one cycle
pub(crate) const DEFAULT_MAX_BLOCKS_FETCH: u64 = 10;

pub(crate) struct OlTrackerCtx<TStorage, TOlClient> {
    pub(crate) storage: Arc<TStorage>,
    pub(crate) ol_client: Arc<TOlClient>,
    pub(crate) ee_state_tx: watch::Sender<EeAccountState>,
    pub(crate) max_blocks_fetch: u64,
}

pub(crate) async fn ol_tracker_task<TStorage, TOlClient>(
    mut state: OlTrackerState,
    ctx: OlTrackerCtx<TStorage, TOlClient>,
) where
    TStorage: Storage,
    TOlClient: OlClient,
{
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;

        match track_ol_state(&state, ctx.ol_client.as_ref(), ctx.max_blocks_fetch).await {
            Ok(TrackOlAction::Extend(block_operations)) => {
                if let Err(error) =
                    handle_extend_ee_state(&block_operations, &mut state, &ctx).await
                {
                    error!(%error, "failed to extend ee state");
                }
            }
            Ok(TrackOlAction::Reorg) => {
                handle_reorg(&mut state, &ctx).await;
            }
            Ok(TrackOlAction::Noop) => {}
            Err(error) => {
                error!(%error, "failed to track ol state");
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct OlBlockOperations {
    pub(crate) block: OLBlockCommitment,
    pub(crate) operations: Vec<UpdateOperationUnconditionalData>,
}

#[derive(Debug)]
pub(crate) enum TrackOlAction {
    /// Extend local view of the OL chain with new blocks.
    Extend(Vec<OlBlockOperations>),
    /// Local tip not present in OL chain, need to resolve local view.
    Reorg,
    /// Local tip is synced with OL chain, nothing to do.
    Noop,
}

pub(crate) async fn track_ol_state(
    state: &OlTrackerState,
    ol_client: &impl OlClient,
    max_blocks_fetch: u64,
) -> eyre::Result<TrackOlAction> {
    let ol_status = ol_client.chain_status().await?;

    let best_ol_block = &ol_status.latest;

    debug!(
        local_slot = state.ol_block.slot(),
        ol_slot = best_ol_block.slot(),
        "check best ol block"
    );

    if best_ol_block.slot() < state.ol_block.slot() {
        // local view of chain is ahead of Ol, should not typically happen
        return Ok(TrackOlAction::Noop);
    }
    if best_ol_block.slot() == state.ol_block.slot() {
        return if best_ol_block.blkid() != state.ol_block.blkid() {
            warn!(slot = best_ol_block.slot(), ol = %best_ol_block.blkid(), local = %state.ol_block.blkid(), "detect chain mismatch; trigger reorg");
            Ok(TrackOlAction::Reorg)
        } else {
            Ok(TrackOlAction::Noop)
        };
    }

    // local chain is behind ol's view, we can fetch next blocks and extend local view.
    let fetch_blocks_count = best_ol_block
        .slot()
        .saturating_sub(state.ol_block.slot())
        .min(max_blocks_fetch);

    // Fetch block commitments in from current local slot.
    // Also fetch height of last known local block to check for reorg.
    let blocks = ol_client
        .block_commitments_in_range_checked(
            state.ol_block.slot(),
            state.ol_block.slot() + fetch_blocks_count,
        )
        .await?;

    let (expected_local_block, new_blocks) = blocks
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty block commitments returned from ol_client"))?;

    // If last block isnt as expected, trigger reorg
    if expected_local_block != &state.ol_block {
        return Ok(TrackOlAction::Reorg);
    }

    let block_ids = new_blocks
        .iter()
        .map(|commitment| commitment.blkid())
        .cloned()
        .collect();

    let operations = ol_client
        .get_update_operations_for_blocks_checked(block_ids)
        .await?;

    let res = new_blocks
        .iter()
        .cloned()
        .zip(operations)
        .map(|(block, operations)| OlBlockOperations { block, operations })
        .collect();

    Ok(TrackOlAction::Extend(res))
}

pub(crate) fn apply_block_operations(
    state: &mut EeAccountState,
    block_operations: &[UpdateOperationUnconditionalData],
) -> eyre::Result<()> {
    for op in block_operations {
        apply_update_operation_unconditionally(state, op)?;
    }

    Ok(())
}

/// Pure function to update tracker state with new block and ee state.
pub(crate) fn update_tracker_state(
    state: &mut OlTrackerState,
    ol_block: OLBlockCommitment,
    ee_state: EeAccountState,
) {
    state.ol_block = ol_block;
    state.ee_state = ee_state;
}

/// Notify watchers of state update.
pub(crate) fn notify_state_update(
    sender: &watch::Sender<EeAccountState>,
    state: &EeAccountState,
) {
    let _ = sender.send(state.clone());
}

async fn handle_extend_ee_state<TStorage, TOlClient>(
    block_operations: &[OlBlockOperations],
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) -> eyre::Result<()>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    for block_op in block_operations {
        let OlBlockOperations {
            block: ol_block,
            operations,
        } = block_op;

        let mut ee_state = state.ee_state.clone();

        // 1. Apply all operations in the block to update local ee account state.
        apply_block_operations(&mut ee_state, &operations).map_err(|error| {
            error!(
                slot = ol_block.slot(),
                %error,
                "failed to apply ol block operation"
            );
            error
        })?;

        // 2. Persist corresponding ee state for every ol block
        ctx.storage
            .store_ee_account_state(ol_block, &ee_state)
            .await
            .map_err(|error| {
                error!(
                    slot = ol_block.slot(),
                    %error,
                    "failed to store ee account state"
                );
                error
            })?;

        // 3. update local state
        update_tracker_state(state, *ol_block, ee_state.clone());

        // 4. notify watchers
        notify_state_update(&ctx.ee_state_tx, &ee_state);
    }

    Ok(())
}

async fn handle_reorg<TStorage, TOlClient>(
    _state: &mut OlTrackerState,
    _ctx: &OlTrackerCtx<TStorage, TOlClient>,
) where
    TStorage: Storage,
    TOlClient: OlClient,
{
    warn!("handle reorg");
    todo!()
}
