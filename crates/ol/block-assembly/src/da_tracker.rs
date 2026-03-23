use std::{collections::HashMap, sync::Arc};

use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId};
use strata_ledger_types::{IAccountStateMut, IStateAccessor};
use strata_ol_chain_types_new::{OLBlock, OLLog};
use strata_ol_state_support_types::{DaAccumulatingState, EpochDaAccumulator};
use strata_ol_stf::execute_block_batch;

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

/// Walks backward from `from_blkid` collecting blocks until a terminal block or genesis.
///
/// Returns blocks in forward chronological order and the boundary header
/// (the terminal/genesis block that precedes the epoch).
async fn collect_epoch_blocks_until<C: BlockAssemblyAnchorContext>(
    from_id: OLBlockId,
    epoch: Epoch,
    ctx: &C,
) -> Result<Vec<OLBlock>, BlockAssemblyError> {
    let mut blocks = Vec::new();
    let mut cur_id = from_id;

    loop {
        let block = ctx
            .fetch_ol_block(cur_id)
            .await?
            .ok_or(BlockAssemblyError::BlockNotFound(cur_id))?;

        let parent_id = *block.header().parent_blkid();

        // Fetch parent to check if it's the epoch boundary.
        let parent_block = ctx
            .fetch_ol_block(parent_id)
            .await?
            .ok_or(BlockAssemblyError::BlockNotFound(parent_id))?;

        if parent_block.header().is_terminal() || parent_block.header().is_genesis_slot() {
            blocks.reverse();
            return Ok(blocks);
        }

        // Check if the same epoch is being traversed through
        if parent_block.header().epoch() != epoch {
            return Err(BlockAssemblyError::Other(
                "Previous epoch without encountering terminal block".to_string(),
            ));
        }

        blocks.push(block);
        cur_id = parent_id;
    }
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
    if epoch_blocks.is_empty() {
        // TODO: better errors
        Err(BlockAssemblyError::Other("Empty epoch blocks".to_string()))
    } else {
        let start_blk = epoch_blocks.first().unwrap();
        let initial_state = fetch_ol_state(start_blk, ctx).await?;

        let mut da_state = DaAccumulatingState::new(Arc::unwrap_or_clone(initial_state));
        let batch_logs = execute_block_batch(&mut da_state, &epoch_blocks, start_blk.header())
            .map_err(|e| BlockAssemblyError::Other(format!("epoch block replay failed: {e}")))?;

        let epoch = epoch_blocks
            .first()
            .expect("epoch_blocks is non-empty")
            .header()
            .epoch();

        let (accumulator, _) = da_state.into_parts();

        Ok(AccumulatedDaData::new(epoch, accumulator, batch_logs))
    }
}

async fn fetch_ol_state<C: BlockAssemblyAnchorContext>(
    blk: &OLBlock,
    ctx: &C,
) -> Result<Arc<C::State>, BlockAssemblyError> {
    let blkid = blk.header().compute_block_commitment();
    // TODO: the context should not return 'Arc'ed state, should return just the state.
    let ol_state = ctx.fetch_state_for_tip(blkid).await?.ok_or_else(|| {
        BlockAssemblyError::Other(format!("missing OL state at epoch boundary {blkid}"))
    })?;
    Ok(ol_state)
}

#[derive(Clone, Debug)]
pub(crate) struct AccumulatedDaData {
    epoch: Epoch,
    accumulator: EpochDaAccumulator,
    logs: Vec<OLLog>,
}

impl AccumulatedDaData {
    pub(crate) fn new_empty(epoch: Epoch) -> Self {
        Self::new(epoch, EpochDaAccumulator::default(), Vec::default())
    }

    pub(crate) fn new(epoch: Epoch, accumulator: EpochDaAccumulator, logs: Vec<OLLog>) -> Self {
        Self {
            epoch,
            accumulator,
            logs,
        }
    }

    pub(crate) fn into_parts(self) -> (EpochDaAccumulator, Vec<OLLog>) {
        (self.accumulator, self.logs)
    }
}
