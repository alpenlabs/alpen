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

/// Number of Ol blocks to process in one cycle
const MAX_BLOCKS_FETCH: u64 = 10;

pub(crate) struct OlTrackerCtx<TStorage, TOlClient> {
    pub(crate) storage: Arc<TStorage>,
    pub(crate) ol_client: Arc<TOlClient>,
    pub(crate) ee_state_tx: watch::Sender<EeAccountState>,
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

        match track_ol_state(&state, ctx.ol_client.as_ref()).await {
            Ok(TrackOlAction::Extend(block_operations)) => {
                handle_extend_ee_state(&block_operations, &mut state, &ctx).await;
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
struct OlBlockOperations {
    block: OLBlockCommitment,
    operations: Vec<UpdateOperationUnconditionalData>,
}

#[derive(Debug)]
enum TrackOlAction {
    /// Extend local view of the OL chain with new blocks.
    Extend(Vec<OlBlockOperations>),
    /// Local tip not present in OL chain, need to resolve local view.
    Reorg,
    /// Local tip is synced with OL chain, nothing to do.
    Noop,
}

async fn track_ol_state(
    state: &OlTrackerState,
    ol_client: &impl OlClient,
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
        .min(MAX_BLOCKS_FETCH);

    // Fetch block commitments in from current local slot.
    // Also fetch height of last known local block to check for reorg.
    let blocks = ol_client
        .block_commitments_in_range(
            state.ol_block.slot(),
            state.ol_block.slot() + fetch_blocks_count,
        )
        .await?;

    let (expected_local_block, new_blocks) =
        blocks.split_first().expect("non empty block commitments");

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
        .get_update_operations_for_blocks(block_ids)
        .await?;

    let res = new_blocks
        .iter()
        .cloned()
        .zip(operations)
        .map(|(block, operations)| OlBlockOperations { block, operations })
        .collect();

    Ok(TrackOlAction::Extend(res))
}

fn apply_block_operations(
    state: &mut EeAccountState,
    block_operations: &[UpdateOperationUnconditionalData],
) -> eyre::Result<()> {
    for op in block_operations {
        apply_update_operation_unconditionally(state, op)?;
    }

    Ok(())
}

async fn handle_extend_ee_state<TStorage, TOlClient>(
    block_operations: &[OlBlockOperations],
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) where
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
        if let Err(error) = apply_block_operations(&mut ee_state, &operations) {
            error!(
                slot = ol_block.slot(),
                %error,
                "failed to apply ol block operation"
            );
            return;
        }
        // 2. Persist corresponding ee state for every ol block
        if let Err(error) = ctx
            .storage
            .store_ee_account_state(ol_block, &ee_state)
            .await
        {
            error!(
                slot = ol_block.slot(),
                %error,
                "failed to store ee account state"
            );
            return;
        }
        // 3. update local state
        state.ee_state = ee_state;
        // TODO: notify state update
    }
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
