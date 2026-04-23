use std::{collections::HashMap, sync::Arc};

use strata_identifiers::{Epoch, OLBlockCommitment, OLBlockId};
use strata_ledger_types::{IAccountStateMut, IStateAccessorMut};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLLog};
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

    /// Removes accumulated DA for a block id, if present.
    pub(crate) fn remove_accumulated_da(&mut self, blkid: OLBlockId) {
        self.block_da_map.remove(&blkid);
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

#[derive(Clone, Debug)]
pub(crate) struct EpochBlocks {
    pub(crate) blocks: NonEmptyVec<OLBlock>,
    pub(crate) epoch_parent: OLBlockHeader,
}

/// Walks backward from `from_blkid` collecting blocks until a terminal block or genesis. Errors
/// out when epoch number is different from the input epoch number.
///
/// Returns blocks in forward chronological order.
async fn collect_epoch_blocks_until<C: BlockAssemblyAnchorContext>(
    target_id: OLBlockId,
    epoch: Epoch,
    ctx: &C,
) -> Result<EpochBlocks, BlockAssemblyError> {
    if epoch == 0 {
        return Err(BlockAssemblyError::GenesisEpochNoBoundary);
    }

    let mut blocks = Vec::new();
    let mut cur_id = target_id;

    let epoch_parent = loop {
        let block = ctx
            .fetch_ol_block(cur_id)
            .await?
            .ok_or(BlockAssemblyError::BlockNotFound(cur_id))?;

        // Block doesn't belong to our epoch — must be the boundary.
        if block.header().epoch() != epoch {
            if !block.header().is_terminal() || block.header().epoch() != epoch - 1 {
                return Err(BlockAssemblyError::InvalidEpochBoundary {
                    blkid: block.header().compute_blkid(),
                    expected_prev_epoch: epoch - 1,
                    got_epoch: block.header().epoch(),
                    is_terminal: block.header().is_terminal(),
                });
            }
            break block.header().clone();
        }

        let parent_id = *block.header().parent_blkid();
        blocks.push(block);
        cur_id = parent_id;
    };

    blocks.reverse();

    let blocks = NonEmptyVec::try_from_vec(blocks)
        .map_err(|_| BlockAssemblyError::Other("Empty epoch blocks".to_string()))?;

    let epoch_blocks = EpochBlocks {
        blocks,
        epoch_parent,
    };
    Ok(epoch_blocks)
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
    <C::State as IStateAccessorMut>::AccountStateMut: Clone,
    <<C::State as IStateAccessorMut>::AccountStateMut as IAccountStateMut>::SnarkAccountStateMut:
        Clone,
{
    let epoch_blocks = collect_epoch_blocks_until(blkid.blkid, epoch, ctx).await?;
    let initial_state = fetch_state(&epoch_blocks.epoch_parent, ctx).await?;

    let mut da_state = DaAccumulatingState::new(Arc::unwrap_or_clone(initial_state));
    let batch_logs = execute_block_batch(
        &mut da_state,
        &epoch_blocks.blocks,
        &epoch_blocks.epoch_parent,
    )
    .map_err(|e| BlockAssemblyError::Other(format!("epoch block replay failed: {e}")))?;

    let (accumulator, _) = da_state.into_parts();

    Ok(AccumulatedDaData::new(accumulator, batch_logs))
}

/// Fetches the state for `blk_header`.
async fn fetch_state<C: BlockAssemblyAnchorContext>(
    blk_header: &OLBlockHeader,
    ctx: &C,
) -> Result<Arc<C::State>, BlockAssemblyError> {
    let blkid = blk_header.compute_block_commitment();
    let ol_state = ctx
        .fetch_state_for_tip(blkid)
        .await?
        .ok_or(BlockAssemblyError::EpochBoundaryStateNotFound(blkid))?;
    Ok(ol_state)
}

/// An 'append-only' container of state diff and OL logs accumulated DA data for some epoch.
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

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, Buf64, OLBlockId};
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };

    use super::*;
    use crate::test_utils::{TestEnv, TestStorageFixtureBuilder};

    async fn build_test_env() -> TestEnv {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new().build_fixture().await;
        TestEnv::from_fixture(fixture, parent_commitment)
    }

    fn make_block(
        slot: u64,
        epoch: Epoch,
        is_terminal: bool,
        parent_blkid: OLBlockId,
        timestamp: u64,
    ) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(is_terminal);
        let header = OLBlockHeader::new(
            timestamp,
            flags,
            slot,
            epoch,
            parent_blkid,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
        OLBlock::new(signed_header, body)
    }

    fn test_blkid(seed: u8) -> OLBlockId {
        OLBlockId::from(Buf32::from([seed; 32]))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_collect_epoch_blocks_epoch_zero() {
        let env = build_test_env().await;

        let err = collect_epoch_blocks_until(test_blkid(1), 0, env.ctx())
            .await
            .expect_err("epoch 0 should be rejected");
        assert!(
            matches!(err, BlockAssemblyError::GenesisEpochNoBoundary),
            "expected GenesisEpochNoBoundary, got: {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_collect_epoch_blocks_invalid_boundary() {
        let env = build_test_env().await;

        // epoch-2 target points to an epoch-1 non-terminal block, which is an invalid boundary.
        let boundary = make_block(1, 1, false, OLBlockId::null(), 1_000_001);
        let target = make_block(2, 2, false, boundary.header().compute_blkid(), 1_000_002);

        env.put_block(boundary).await;
        env.put_block(target.clone()).await;

        let err = collect_epoch_blocks_until(target.header().compute_blkid(), 2, env.ctx())
            .await
            .expect_err("invalid boundary should fail");
        assert!(
            matches!(
                err,
                BlockAssemblyError::InvalidEpochBoundary {
                    expected_prev_epoch: 1,
                    got_epoch: 1,
                    is_terminal: false,
                    ..
                }
            ),
            "expected InvalidEpochBoundary, got: {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_rebuild_da_missing_boundary_state() {
        let env = build_test_env().await;

        // Valid boundary shape (terminal epoch-1), but boundary OL state is intentionally absent.
        let boundary = make_block(10, 1, true, OLBlockId::null(), 1_000_010);
        let target = make_block(11, 2, false, boundary.header().compute_blkid(), 1_000_011);
        let target_commitment =
            OLBlockCommitment::new(target.header().slot(), target.header().compute_blkid());

        env.put_block(boundary).await;
        env.put_block(target).await;

        let err = rebuild_accumulated_da_upto(target_commitment, 2, env.ctx())
            .await
            .expect_err("missing boundary state should fail rebuild");
        assert!(
            matches!(err, BlockAssemblyError::EpochBoundaryStateNotFound(_)),
            "expected EpochBoundaryStateNotFound(_), got: {err:?}"
        );
    }

    #[test]
    fn test_tracker_set_remove_parent_entry() {
        let parent = test_blkid(10);
        let child = test_blkid(11);
        let unrelated = test_blkid(12);

        let mut tracker = EpochDaTracker::new_empty();
        tracker.set_accumulated_da(parent, AccumulatedDaData::new_empty());
        tracker.set_accumulated_da(unrelated, AccumulatedDaData::new_empty());
        tracker.set_accumulated_da_and_remove_parent_entry(
            child,
            parent,
            AccumulatedDaData::new_empty(),
        );

        assert!(
            tracker.get_accumulated_da(parent).is_none(),
            "parent entry must be removed"
        );
        assert!(
            tracker.get_accumulated_da(child).is_some(),
            "child entry must be inserted"
        );
        assert!(
            tracker.get_accumulated_da(unrelated).is_some(),
            "unrelated entries must remain"
        );
    }
}
