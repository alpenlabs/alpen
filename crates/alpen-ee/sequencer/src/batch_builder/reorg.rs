//! Batch builder reorg related logic.

use alpen_ee_common::{Batch, BatchId, BatchStorage, BlockNumHash};
use eyre::{eyre, Result};
use tracing::{error, warn};

use super::{canonical::CanonicalChainReader, BatchBuilderState, BatchPolicy};

/// Find the last batch whose end block is still canonical.
/// If a canonical batch cannot be found at height > finalized height, it is a deep reorg and must
/// be handled manually.
async fn find_last_canonical_unfinalized_batch(
    canonical_reader: &impl CanonicalChainReader,
    batch_storage: &impl BatchStorage,
) -> Result<Option<Batch>> {
    let (batch, _) = batch_storage.get_latest_batch().await?;

    // TODO: get this directly
    let finalized_blocknum = canonical_reader.finalized_blocknum().await?;

    let mut idx = batch.idx();
    loop {
        let (batch, _) = batch_storage
            .get_batch_by_idx(idx)
            .await?
            .ok_or_else(|| eyre!("missing batch data: {idx}"))?;

        if batch.last_blocknum() < finalized_blocknum {
            return Ok(None);
        }

        if canonical_reader.is_canonical(batch.last_block()).await? {
            return Ok(Some(batch));
        }

        if idx == 0 {
            error!("Genesis batch is not canonical. Ensure config is valid.");
            return Ok(None);
        }
        idx -= 1;
    }
}

pub(crate) enum ReorgReport {
    /// No reorg detected, latest accumulator and pending blocks queue is canonical.
    NoReorg,
    /// Reorg in accumulator or pending blocks queue.
    ShallowReorg,
    /// Batch reorg detected, returning new latest batch id that is still canonical.
    Reorg(BatchId),
    /// Reorg is too deep and cannot be handled automatically. Need manual intervention.
    DeepReorg,
}

/// Check for reorgs and handle them.
///
/// Returns a [`ReorgReport`] indicating the type of reorg detected:
/// - `NoReorg` - the tip is canonical, no action needed
/// - `ShallowReorg` - only accumulated/pending blocks were reorged, state was reset
/// - `Reorg(BatchId)` - batches were reverted, returning the new latest batch id
/// - `DeepReorg` - reorg below finalized batches, manual intervention required
pub(crate) async fn check_and_handle_reorg<P: BatchPolicy>(
    state: &mut BatchBuilderState<P>,
    canonical_reader: &impl CanonicalChainReader,
    batch_storage: &impl BatchStorage,
    genesis_block: BlockNumHash,
) -> Result<ReorgReport> {
    // 0. edge case: we have no batches and are tracking no blocks.
    if state.last_known_block() == genesis_block {
        return Ok(ReorgReport::NoReorg);
    }

    // 1. Check latest block - no reorg if canonical
    if canonical_reader
        .is_canonical(state.last_known_block().hash())
        .await?
    {
        return Ok(ReorgReport::NoReorg);
    }
    // latest block is NOT canonical

    // 2. Check last sealed batch is still canonical
    if canonical_reader
        .is_canonical(state.prev_batch_end().hash())
        .await?
    {
        // NOTE: Ignores the case where only the pending blocks are re-org'd but accumulated blocks
        // are still canonical. Just reset everything not in a batch already and rebuild the state.

        // clear accumulator and pending blocks
        state.accumulator_mut().reset();
        state.clear_pending_blocks();
        warn!("Shallow reorg detected, reset accumulator and pending blocks");
        return Ok(ReorgReport::ShallowReorg);
    }

    // 3. Batch reorg - find last canonical unfinalize batchd and revert everything after it
    let Some(batch) =
        find_last_canonical_unfinalized_batch(canonical_reader, batch_storage).await?
    else {
        error!("Cannot revert finalized batches. Manual intervention required.");
        return Ok(ReorgReport::DeepReorg);
    };

    batch_storage.revert_batches(batch.idx()).await?;
    *state = BatchBuilderState::from_last_batch(batch.idx(), batch.last_blocknumhash());
    warn!(
        reverted_to_idx = batch.idx(),
        "Deep reorg detected, reverted batches"
    );
    Ok(ReorgReport::Reorg(batch.id()))
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{Batch, BatchStatus, MockBatchStorage};

    use super::*;
    use crate::{
        batch_builder::{canonical::MockCanonicalChainReader, BlockCountData, BlockCountPolicy},
        test_utils::*,
    };

    fn make_batch(idx: u64, prev_block: BlockNumHash, last_block: BlockNumHash) -> Batch {
        Batch::new(
            idx,
            prev_block.hash(),
            last_block.hash(),
            last_block.blocknum(),
            vec![],
        )
        .unwrap()
    }

    fn make_genesis_batch(block: BlockNumHash) -> Batch {
        Batch::new_genesis_batch(block.hash(), block.blocknum()).unwrap()
    }

    mod no_reorg_tests {
        use super::*;

        #[tokio::test]
        async fn returns_no_reorg_when_tip_is_canonical_with_empty_accumulator() {
            // Scenario: Empty accumulator, prev_batch_end is canonical
            // Batches:      [genesis]
            // Accumulator:  []
            // Canonical:    genesis
            // Expected:     No reorg

            let genesis = test_blocknumhash(0);
            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(0, genesis);

            let mut canonical = MockCanonicalChainReader::new();
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == genesis.hash())
                .returning(|_| Ok(true));

            let batch_storage = MockBatchStorage::new();

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::NoReorg));
        }

        #[tokio::test]
        async fn returns_no_reorg_when_accumulator_tip_is_canonical() {
            // Scenario: Accumulator has blocks, tip is canonical
            // Batches:      [genesis]
            // Accumulator:  [block1, block2]
            // Canonical:    block2
            // Expected:     No reorg

            let genesis = test_blocknumhash(0);
            let block1 = test_blocknumhash(1);
            let block2 = test_blocknumhash(2);

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(0, genesis);
            state.accumulator_mut().add_block(block1, &BlockCountData);
            state.accumulator_mut().add_block(block2, &BlockCountData);

            let mut canonical = MockCanonicalChainReader::new();
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == block2.hash())
                .returning(|_| Ok(true));

            let batch_storage = MockBatchStorage::new();

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::NoReorg));
        }
    }

    mod shallow_reorg_tests {
        use super::*;

        #[tokio::test]
        async fn clears_accumulator_when_prev_batch_end_canonical_but_accumulator_not() {
            // Scenario: Accumulator tip reorged out, but prev_batch_end still canonical
            // Batches:      [genesis]
            // Accumulator:  [block1, block2] (reorged out)
            // Pending:      [block3, block4]
            // Canonical:    genesis (but not block4 which is last_known_block)
            // Expected:     Shallow reorg - clear accumulator, return ShallowReorg

            let genesis = test_blocknumhash(0);
            let block1 = test_blocknumhash(1);
            let block2 = test_blocknumhash(2);
            let block3 = test_blocknumhash(3);
            let block4 = test_blocknumhash(4);

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(0, genesis);
            state.accumulator_mut().add_block(block1, &BlockCountData);
            state.accumulator_mut().add_block(block2, &BlockCountData);
            state.push_pending_blocks(vec![block3, block4]);

            let mut canonical = MockCanonicalChainReader::new();
            // First check: last_known_block (block4) is not canonical
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == block4.hash())
                .times(1)
                .returning(|_| Ok(false));
            // Second check: prev_batch_end (genesis) is canonical
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == genesis.hash())
                .times(1)
                .returning(|_| Ok(true));

            let batch_storage = MockBatchStorage::new();

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::ShallowReorg));
            assert!(state.accumulator().is_empty());
            assert!(!state.has_pending_blocks());
        }
    }

    mod batch_reorg_tests {
        use super::*;

        #[tokio::test]
        async fn reverts_to_genesis_batch_when_only_genesis_canonical() {
            // Scenario: Batch reorg, only genesis batch is canonical
            // Batches:      [genesis_batch] -> [batch1] (batch1 reorged)
            // Canonical:    genesis
            // Expected:     Batch reorg - revert to genesis_batch, return Reorg(genesis_batch_id)

            let genesis = test_blocknumhash(0);
            let batch1_end = test_blocknumhash(20);

            let genesis_batch = make_genesis_batch(genesis);
            let genesis_batch_id = genesis_batch.id();

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(1, batch1_end);

            let mut canonical = MockCanonicalChainReader::new();
            // batch1_end is not canonical (checked twice: once for last_known_block, once for
            // prev_batch_end)
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == batch1_end.hash())
                .times(2)
                .returning(|_| Ok(false));
            // genesis is canonical
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == genesis.hash())
                .times(1)
                .returning(|_| Ok(true));
            canonical.expect_finalized_blocknum().returning(|| Ok(0));

            let genesis_batch_for_latest = make_genesis_batch(genesis);
            let genesis_batch_for_idx = make_genesis_batch(genesis);
            let batch1_for_idx = make_batch(1, genesis, batch1_end);

            let mut batch_storage = MockBatchStorage::new();
            batch_storage
                .expect_get_latest_batch()
                .returning(move || Ok((genesis_batch_for_latest.clone(), BatchStatus::Sealed)));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 1)
                .returning(move |_| Ok(Some((batch1_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 0)
                .returning(move |_| Ok(Some((genesis_batch_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_revert_batches()
                .withf(|idx| *idx == 0)
                .times(1)
                .returning(|_| Ok(()));

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::Reorg(id) if id == genesis_batch_id));
            assert_eq!(state.prev_batch_end(), genesis);
            assert_eq!(state.next_batch_idx(), 1);
        }

        #[tokio::test]
        async fn reverts_to_last_canonical_batch() {
            // Scenario: Batch reorg, batch1 is last canonical batch (after genesis)
            // Batches:      [genesis_batch] -> [batch1] -> [batch2] (batch2 reorged)
            // Canonical:    batch1_end
            // Expected:     Batch reorg - revert to batch1, return Reorg(batch1_id)

            let genesis = test_blocknumhash(0);
            let batch0_end = test_blocknumhash(10);
            let batch1_end = test_blocknumhash(20);
            let batch2_end = test_blocknumhash(30);

            let batch1 = make_batch(1, batch0_end, batch1_end);
            let batch1_id = batch1.id();

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(2, batch2_end);

            let mut canonical = MockCanonicalChainReader::new();
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == batch2_end.hash())
                .times(3)
                .returning(|_| Ok(false));
            canonical
                .expect_is_canonical()
                .withf(move |h| *h == batch1_end.hash())
                .times(1)
                .returning(|_| Ok(true));
            canonical.expect_finalized_blocknum().returning(|| Ok(0)); // Nothing finalized yet

            let batch2_for_latest = make_batch(2, batch1_end, batch2_end);
            let batch2_for_idx = make_batch(2, batch1_end, batch2_end);
            let batch1_for_idx = make_batch(1, batch0_end, batch1_end);

            let mut batch_storage = MockBatchStorage::new();
            batch_storage
                .expect_get_latest_batch()
                .returning(move || Ok((batch2_for_latest.clone(), BatchStatus::Sealed)));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 2)
                .returning(move |_| Ok(Some((batch2_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 1)
                .returning(move |_| Ok(Some((batch1_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_revert_batches()
                .withf(|idx| *idx == 1)
                .times(1)
                .returning(|_| Ok(()));

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::Reorg(id) if id == batch1_id));
            assert_eq!(state.prev_batch_end(), batch1_end);
            assert_eq!(state.next_batch_idx(), 2);
        }
    }

    mod deep_reorg_tests {
        use super::*;

        #[tokio::test]
        async fn returns_deep_reorg_when_no_batches_canonical() {
            // Scenario: Deep reorg, no batches are canonical (reorg below genesis batch)
            // Batches:      [genesis_batch] -> [batch1] (all reorged)
            // Canonical:    none (even genesis batch is not canonical)
            // Expected:     DeepReorg - manual intervention required

            let genesis = test_blocknumhash(0);
            let batch1_end = test_blocknumhash(20);

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(1, batch1_end);

            let mut canonical = MockCanonicalChainReader::new();
            canonical.expect_is_canonical().returning(|_| Ok(false));
            canonical.expect_finalized_blocknum().returning(|| Ok(0)); // Nothing finalized

            let genesis_batch_for_idx = make_genesis_batch(genesis);
            let batch1_for_latest = make_batch(1, genesis, batch1_end);
            let batch1_for_idx = make_batch(1, genesis, batch1_end);

            let mut batch_storage = MockBatchStorage::new();
            batch_storage
                .expect_get_latest_batch()
                .returning(move || Ok((batch1_for_latest.clone(), BatchStatus::Sealed)));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 1)
                .returning(move |_| Ok(Some((batch1_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 0)
                .returning(move |_| Ok(Some((genesis_batch_for_idx.clone(), BatchStatus::Sealed))));

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::DeepReorg));
        }

        #[tokio::test]
        async fn returns_deep_reorg_when_reorg_below_finalized() {
            // Scenario: Reorg below finalized block height
            // Batches:      [genesis_batch (ends at 10)] -> [batch1 (ends at 20)] (both not
            // canonical) Finalized:    block 15 (above genesis batch end)
            // Expected:     DeepReorg - cannot revert finalized batches
            //
            // Flow: batch1 not canonical, check batch0, batch0.last_blocknum (10) < finalized (15)
            //       → return None → DeepReorg

            let genesis = test_blocknumhash(0);
            let batch1_end = test_blocknumhash(20);

            let mut state: BatchBuilderState<BlockCountPolicy> =
                BatchBuilderState::from_last_batch(1, batch1_end);

            let mut canonical = MockCanonicalChainReader::new();
            canonical.expect_is_canonical().returning(|_| Ok(false));
            canonical.expect_finalized_blocknum().returning(|| Ok(15)); // Finalized above genesis batch end

            let genesis_batch_for_idx = make_genesis_batch(genesis);
            let batch1_for_latest = make_batch(1, genesis, batch1_end);
            let batch1_for_idx = make_batch(1, genesis, batch1_end);

            let mut batch_storage = MockBatchStorage::new();
            batch_storage
                .expect_get_latest_batch()
                .returning(move || Ok((batch1_for_latest.clone(), BatchStatus::Sealed)));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 1)
                .returning(move |_| Ok(Some((batch1_for_idx.clone(), BatchStatus::Sealed))));
            batch_storage
                .expect_get_batch_by_idx()
                .withf(|idx| *idx == 0)
                .returning(move |_| Ok(Some((genesis_batch_for_idx.clone(), BatchStatus::Sealed))));

            let result = check_and_handle_reorg(&mut state, &canonical, &batch_storage, genesis)
                .await
                .unwrap();

            assert!(matches!(result, ReorgReport::DeepReorg));
        }
    }
}
