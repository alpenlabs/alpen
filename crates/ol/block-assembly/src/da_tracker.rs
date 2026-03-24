use std::{collections::HashMap, sync::Arc};

use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId};
use strata_ledger_types::{IAccountStateMut, IStateAccessor};
use strata_ol_chain_types_new::{OLBlock, OLLog};
use strata_ol_state_support_types::{DaAccumulatingState, EpochDaAccumulator};
use strata_ol_stf::execute_block_batch;
use strata_primitives::nonempty_vec::NonEmptyVec;

use crate::{BlockAssemblyAnchorContext, BlockAssemblyError, BlockAssemblyStateAccess};

#[derive(Clone, Debug)]
pub(crate) struct EpochDaTracker {
    block_da_map: HashMap<OLBlockId, AccumulatedDaData>,
}

impl EpochDaTracker {
    pub(crate) fn new_empty() -> Self {
        Self::new(HashMap::default())
    }

    pub(crate) fn new(block_da_map: HashMap<OLBlockId, AccumulatedDaData>) -> Self {
        Self { block_da_map }
    }

    pub(crate) fn get_accumulated_da(&self, blkid: OLBlockId) -> Option<&AccumulatedDaData> {
        self.block_da_map.get(&blkid)
    }

    pub(crate) fn set_accumulated_da(&mut self, blkid: OLBlockId, da: AccumulatedDaData) {
        self.block_da_map.insert(blkid, da);
    }

    /// Inserts the entry for given block id and also removes the entry for parent if exists. This
    /// method is used to optimize memory usage because in the next assembly we would require
    /// accumulation upto the current block and not the parent block.
    pub(crate) fn set_accumulated_da_and_remove_parent_entry(
        &mut self,
        blkid: OLBlockId,
        parent: OLBlockId,
        da: AccumulatedDaData,
    ) {
        self.set_accumulated_da(blkid, da);
        self.block_da_map.remove(&parent);
    }
}

/// Walks backward from `from_blkid` collecting blocks until a terminal block or genesis. Errors
/// out when epoch number is different from the input epoch number.
///
/// Returns blocks in forward chronological order.
async fn collect_epoch_blocks_until<C: BlockAssemblyAnchorContext>(
    target_id: OLBlockId,
    epoch: Epoch,
    ctx: &C,
) -> Result<NonEmptyVec<OLBlock>, BlockAssemblyError> {
    let mut blocks = Vec::new();
    let mut cur_id = target_id;

    loop {
        let block = ctx
            .fetch_ol_block(cur_id)
            .await?
            .ok_or(BlockAssemblyError::BlockNotFound(cur_id))?;

        if block.header().is_genesis_slot() {
            // Genesis is the boundary for epoch 0 — we don't collect it, just stop.
            break;
        }

        if block.header().is_terminal() {
            // Terminal block sealed the previous epoch; sanity check.
            if block.header().epoch() != epoch - 1 {
                return Err(BlockAssemblyError::Other(
                    "invalid epoch for previous terminal block".to_string(),
                ));
            }
            break;
        }

        // Sanity check epoch number
        if block.header().epoch() != epoch {
            return Err(BlockAssemblyError::Other(
                "invalid epoch for epochal block".to_string(),
            ));
        }

        let parent_id = *block.header().parent_blkid();

        // Insert block and update current block to be the parent block
        blocks.push(block);
        cur_id = parent_id;
    }

    blocks.reverse();
    let blocks = NonEmptyVec::try_from_vec(blocks)
        .map_err(|_| BlockAssemblyError::Other("Empty epoch blocks".to_string()))?;
    Ok(blocks)
}

/// Rebuilds accumulated DA for `target_blkid` by replaying all epoch blocks
/// from the epoch boundary up to and including `target_blkid`.
pub(crate) async fn rebuild_accumulated_da_upto<C: BlockAssemblyAnchorContext>(
    blkid: OLBlockCommitment,
    epoch: Epoch,
    ctx: &C,
) -> Result<AccumulatedDaData, BlockAssemblyError>
where
    C::State: BlockAssemblyStateAccess,
    <C::State as IStateAccessor>::AccountStateMut: Clone,
    <<C::State as IStateAccessor>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
        Clone,
{
    let epoch_blocks = collect_epoch_blocks_until(blkid.blkid, epoch, ctx).await?;
    let first_blk = epoch_blocks.ensured_first();
    let initial_state = fetch_pre_state(first_blk, ctx).await?;

    let mut da_state = DaAccumulatingState::new(Arc::unwrap_or_clone(initial_state));
    let batch_logs = execute_block_batch(&mut da_state, &epoch_blocks, first_blk.header())
        .map_err(|e| BlockAssemblyError::Other(format!("epoch block replay failed: {e}")))?;

    let (accumulator, _) = da_state.into_parts();

    Ok(AccumulatedDaData::new(accumulator, batch_logs))
}

/// Fetches the pre-state for `blk` by looking up the post-state of its parent.
async fn fetch_pre_state<C: BlockAssemblyAnchorContext>(
    blk: &OLBlock,
    ctx: &C,
) -> Result<Arc<C::State>, BlockAssemblyError> {
    let parent_id = *blk.header().parent_blkid();
    let parent_blk = ctx
        .fetch_ol_block(parent_id)
        .await?
        .ok_or(BlockAssemblyError::BlockNotFound(parent_id))?;
    let parent_commitment = parent_blk.header().compute_block_commitment();
    let ol_state = ctx
        .fetch_state_for_tip(parent_commitment)
        .await?
        .ok_or_else(|| {
            BlockAssemblyError::Other(format!(
                "missing OL state at epoch boundary {parent_commitment}"
            ))
        })?;
    Ok(ol_state)
}

/// Contains accumulated DA data for some epoch which includes state diff accumulator and OL logs.
#[derive(Clone, Debug)]
pub(crate) struct AccumulatedDaData {
    accumulator: EpochDaAccumulator,
    logs: Vec<OLLog>,
}

impl AccumulatedDaData {
    pub(crate) fn new_empty() -> Self {
        Self::new(EpochDaAccumulator::default(), Vec::default())
    }

    pub(crate) fn new(accumulator: EpochDaAccumulator, logs: Vec<OLLog>) -> Self {
        Self { accumulator, logs }
    }

    pub(crate) fn into_parts(self) -> (EpochDaAccumulator, Vec<OLLog>) {
        (self.accumulator, self.logs)
    }

    pub(crate) fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub(crate) fn append_logs(&mut self, new_logs: &[OLLog]) {
        self.logs.extend_from_slice(new_logs);
    }
}
