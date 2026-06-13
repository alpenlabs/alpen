use alpen_ee_common::{
    exec_block_storage_test_fns::create_exec_block, Batch, BatchId, BatchStorage, BlockNumHash,
    Chunk, ChunkStorage, InMemoryStorage, MockExecBlockStorage,
};
use strata_acct_types::Hash;

use super::{
    handlers::{handle_reorg, process_pending},
    recovery::{cleanup_orphaned_chunks, enqueue_backfill, repair_batch_linkage},
    state::{init_chunk_builder_state, ChunkBuilderState, PendingEntry},
};
use crate::sealing_policy::block_count_policy::{
    BlockCountDataProvider, BlockCountPolicy, FixedBlockCountSealing,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_hash(n: u8) -> Hash {
    let mut buf = [0u8; 32];
    buf[0] = 1; // avoid zero hash
    buf[31] = n;
    Hash::from(buf)
}

fn test_block(n: u8) -> BlockNumHash {
    BlockNumHash::new(test_hash(n), n as u64)
}

fn test_batch_id(prev: u8, last: u8) -> BatchId {
    BatchId::from_parts(test_hash(prev), test_hash(last))
}

/// Seed storage with a genesis batch (batch 0, single genesis block).
async fn seed_genesis_batch(storage: &InMemoryStorage, genesis: BlockNumHash) {
    let batch = Batch::new_genesis_batch(genesis.hash(), genesis.blocknum()).unwrap();
    storage.save_genesis_batch(batch).await.unwrap();
}

async fn save_test_batch(storage: &InMemoryStorage, idx: u64, prev: u8, last: u8) -> Batch {
    let inner_blocks = ((prev + 1)..last).map(test_hash).collect();
    let batch = Batch::new(
        idx,
        test_hash(prev),
        test_hash(last),
        last as u64,
        inner_blocks,
    )
    .unwrap();
    storage.save_next_batch(batch.clone()).await.unwrap();
    batch
}

fn mock_block_storage() -> MockExecBlockStorage {
    let mut mock_blocks = MockExecBlockStorage::new();
    mock_blocks.expect_get_exec_block().returning(|hash| {
        let n = hash.as_ref()[31];
        if n == 0 {
            Ok(Some(create_exec_block(0, Hash::zero(), test_hash(0), 0)))
        } else {
            Ok(Some(create_exec_block(
                n as u64,
                test_hash(n - 1),
                test_hash(n),
                n as u64,
            )))
        }
    });
    mock_blocks
}

/// Create chunk builder state starting from genesis.
///
/// Batch 0 is always the genesis batch (single block), so real
/// processing starts at `current_batch_idx = 1`.
fn new_state(genesis: BlockNumHash) -> ChunkBuilderState<BlockCountPolicy> {
    let mut state = ChunkBuilderState::new(genesis);
    state.set_current_batch_idx(1);
    state
}

// -- Entry constructors --

/// A block entry for block number `n` in the given batch.
fn block(n: u8, batch_idx: u64) -> PendingEntry {
    PendingEntry::Block {
        block: test_block(n),
        batch_idx,
    }
}

/// A batch boundary entry.
fn boundary(prev: u8, last: u8) -> PendingEntry {
    PendingEntry::BatchBoundary(test_batch_id(prev, last))
}

/// Enqueue the given entries and drain the pending queue fully.
async fn process_entries(
    state: &mut ChunkBuilderState<BlockCountPolicy>,
    storage: &InMemoryStorage,
    policy: &FixedBlockCountSealing,
    entries: &[PendingEntry],
) {
    for entry in entries {
        state.push_pending(entry.clone());
    }
    // Loop until the queue is drained (process_pending has a per-tick cap).
    while state.has_pending() {
        process_pending::<BlockCountPolicy, FixedBlockCountSealing, BlockCountDataProvider>(
            state,
            storage,
            policy,
            &BlockCountDataProvider,
        )
        .await
        .expect("process_pending failed");
    }
}

// ---------------------------------------------------------------------------
// Multi-chunk batch with remainder at batch boundary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_chunk_batch_with_remainder() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(4); // chunk every 4

    let mut state = new_state(genesis);

    // Batch 1: blocks 1-6, then boundary seals, block 7 starts batch 2.
    // Expected: chunk 0 [1-4] (policy-sealed) + chunk 1 [5-6] (force-sealed).
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            boundary(0, 6),
            block(7, 2),
        ],
    )
    .await;

    let batch_id = test_batch_id(0, 6);
    let chunk_ids = storage.get_batch_chunks(batch_id).await.unwrap().unwrap();
    assert_eq!(chunk_ids.len(), 2, "expected 2 chunks (4 + 2)");

    let (c0, _) = storage.get_chunk_by_idx(0).await.unwrap().unwrap();
    assert_eq!(
        c0.blocks_iter().collect::<Vec<_>>(),
        vec![test_hash(1), test_hash(2), test_hash(3), test_hash(4)]
    );
    assert_eq!(c0.batch_idx(), 1);
    assert_eq!(c0.prev_block(), genesis.hash());

    let (c1, _) = storage.get_chunk_by_idx(1).await.unwrap().unwrap();
    assert_eq!(
        c1.blocks_iter().collect::<Vec<_>>(),
        vec![test_hash(5), test_hash(6)]
    );
    assert_eq!(c1.batch_idx(), 1);
    assert_eq!(c1.prev_block(), c0.last_block());
    assert_eq!(c1.last_block(), test_hash(6));

    // Block 7 is in the accumulator for batch 2.
    assert_eq!(state.accumulator().block_count(), 1);
    assert_eq!(state.accumulator().blocks().first(), Some(&test_block(7)));
    assert_eq!(state.current_batch_idx(), 2);
}

// ---------------------------------------------------------------------------
// Reorg reverts sealed chunks in current batch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reorg_reverts_unsealed_batch_chunks() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(3);
    seed_genesis_batch(&storage, genesis).await;
    let block_storage = mock_block_storage();

    let mut state = new_state(genesis);

    // Batch 1 (unsealed): blocks 1-7 → chunks [1-3] idx=0, [4-6] idx=1.
    // Block 7 remains in accumulator.
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            block(7, 1),
        ],
    )
    .await;

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 1, "should have 2 chunks before reorg");

    // Reorg back to genesis. Genesis batch (idx=0) is the last valid batch.
    handle_reorg(&mut state, &storage, &storage, &block_storage, genesis, 0)
        .await
        .expect("handle_reorg failed");

    assert!(
        storage.get_latest_chunk().await.unwrap().is_none(),
        "all chunks should be reverted"
    );
    assert_eq!(state.next_chunk_idx(), 0);
    assert_eq!(state.current_batch_idx(), 1);
    assert!(state.accumulator().is_empty());
    assert!(state.current_batch_chunks().is_empty());
}

// ---------------------------------------------------------------------------
// Reorg to batch-boundary block
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reorg_to_batch_boundary() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(3);
    seed_genesis_batch(&storage, genesis).await;
    let batch1 = save_test_batch(&storage, 1, 0, 10).await;
    let block_storage = mock_block_storage();

    let mut state = new_state(genesis);

    // Batch 1: blocks 1-10.
    // Chunks: [1-3] idx=0, [4-6] idx=1, [7-9] idx=2. Block 10 in accumulator.
    // Boundary seals → force-seals [10] → chunk idx=3. Blocks 11-14 start batch 2 and
    // create a chunk that must be reverted.
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            block(7, 1),
            block(8, 1),
            block(9, 1),
            block(10, 1),
            boundary(0, 10),
            block(11, 2),
            block(12, 2),
            block(13, 2),
            block(14, 2),
        ],
    )
    .await;

    let chunk_ids = storage
        .get_batch_chunks(batch1.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(chunk_ids.len(), 4);
    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 4, "batch 2 chunk should exist before reorg");

    // Reorg to batch 1's boundary. Batch 2 is unsealed and must be discarded.
    handle_reorg(
        &mut state,
        &storage,
        &storage,
        &block_storage,
        batch1.last_blocknumhash(),
        1,
    )
    .await
    .expect("handle_reorg failed");

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 3, "batch 2 chunk should be reverted");
    assert_eq!(state.next_chunk_idx(), 4);
    assert_eq!(state.current_batch_idx(), 2);
    assert!(state.accumulator().is_empty());
}

#[tokio::test]
async fn reorg_backfills_surviving_batch_when_events_were_dropped() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(10);
    seed_genesis_batch(&storage, genesis).await;
    let batch1 = save_test_batch(&storage, 1, 0, 3).await;
    let batch2 = save_test_batch(&storage, 2, 3, 6).await;
    let block_storage = mock_block_storage();

    // Batch 1 was fully chunked and linked, but batch 2's block/boundary
    // events were still only in memory when the reorg event arrived.
    let chunk0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    let chunk0_id = chunk0.id();
    storage.save_next_chunk(chunk0).await.unwrap();
    storage
        .set_batch_chunks(batch1.id(), vec![chunk0_id])
        .await
        .unwrap();

    let mut state = ChunkBuilderState::from_last_chunk(0, test_block(3), 2);
    state.push_pending(block(4, 2));
    state.push_pending(block(5, 2));
    state.push_pending(block(6, 2));
    state.push_pending(boundary(3, 6));

    handle_reorg(
        &mut state,
        &storage,
        &storage,
        &block_storage,
        batch2.last_blocknumhash(),
        2,
    )
    .await
    .expect("handle_reorg failed");

    // The stale queue was rebuilt from BatchStorage, so the surviving batch 2
    // still gets chunked even though its original events were dropped.
    while state.has_pending() {
        process_pending::<BlockCountPolicy, FixedBlockCountSealing, BlockCountDataProvider>(
            &mut state,
            &storage,
            &policy,
            &BlockCountDataProvider,
        )
        .await
        .expect("process_pending failed");
    }

    let batch2_chunks = storage
        .get_batch_chunks(batch2.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(batch2_chunks.len(), 1);
    let (chunk1, _) = storage
        .get_chunk_by_id(batch2_chunks[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        chunk1.blocks_iter().collect::<Vec<_>>(),
        vec![test_hash(4), test_hash(5), test_hash(6)]
    );
    assert_eq!(chunk1.batch_idx(), 2);
    assert_eq!(state.current_batch_idx(), 3);
}

// ---------------------------------------------------------------------------
// State recovery from storage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn init_state_from_storage() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(3);

    // Batch 1: 7 blocks → chunks [1-3] idx=0, [4-6] idx=1. Block 7 in accumulator.
    let mut state = new_state(genesis);
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            block(7, 1),
        ],
    )
    .await;

    // Simulate restart: init from storage.
    let recovered: ChunkBuilderState<BlockCountPolicy> =
        init_chunk_builder_state(&storage, genesis)
            .await
            .expect("init failed");

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(recovered.next_chunk_idx(), latest.idx() + 1);
    assert_eq!(recovered.prev_chunk_end().hash(), latest.last_block());
    assert_eq!(
        recovered.prev_chunk_end().blocknum(),
        latest.last_blocknum()
    );
    // latest chunk is in batch 1 → current_batch_idx = 2
    assert_eq!(recovered.current_batch_idx(), latest.batch_idx() + 1);
    assert!(recovered.accumulator().is_empty());
}

#[tokio::test]
async fn init_state_empty_storage() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);

    let state: ChunkBuilderState<BlockCountPolicy> = init_chunk_builder_state(&storage, genesis)
        .await
        .expect("init failed");

    assert_eq!(state.next_chunk_idx(), 0);
    assert_eq!(state.prev_chunk_end(), genesis);
    assert_eq!(state.current_batch_idx(), 1);
}

// ---------------------------------------------------------------------------
// Consecutive batch seals — chunk chain continuous across batches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn consecutive_batch_seals() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(3);

    let mut state = new_state(genesis);

    // Batch 1: blocks 1-4. Boundary seals, block 5 starts batch 2.
    // Seals: block 4 triggers policy seal [1-3] → chunk 0.
    //        boundary force-seals [4] → chunk 1.
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            boundary(0, 4),
            block(5, 2),
        ],
    )
    .await;

    let batch1_id = test_batch_id(0, 4);
    let batch1_chunks = storage.get_batch_chunks(batch1_id).await.unwrap().unwrap();
    assert_eq!(batch1_chunks.len(), 2);

    // Batch 2: blocks 5-8. Boundary seals, block 9 starts batch 3.
    // Block 5 is already in accumulator from above.
    // Seals: block 8 triggers policy seal [5-7] → chunk 2.
    //        boundary force-seals [8] → chunk 3.
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(6, 2),
            block(7, 2),
            block(8, 2),
            boundary(4, 8),
            block(9, 3),
        ],
    )
    .await;

    let batch2_id = test_batch_id(4, 8);
    let batch2_chunks = storage.get_batch_chunks(batch2_id).await.unwrap().unwrap();
    assert_eq!(batch2_chunks.len(), 2);

    // Verify continuous chunk chain across batches.
    let (last_of_batch1, _) = storage.get_chunk_by_idx(1).await.unwrap().unwrap();
    let (first_of_batch2, _) = storage.get_chunk_by_idx(2).await.unwrap().unwrap();
    assert_eq!(
        first_of_batch2.prev_block(),
        last_of_batch1.last_block(),
        "chunk chain must be continuous across batch boundary"
    );

    // Batch chunk lists must be disjoint.
    for id in &batch1_chunks {
        assert!(
            !batch2_chunks.contains(id),
            "batch chunk lists must not overlap"
        );
    }

    // Total: 4 chunks across 2 batches.
    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 3);

    // Block 9 is in the accumulator for batch 3.
    assert_eq!(state.accumulator().block_count(), 1);
    assert_eq!(state.current_batch_idx(), 3);
}

// ---------------------------------------------------------------------------
// Cleanup orphaned chunks
// ---------------------------------------------------------------------------

/// Helper: manually insert a chunk into storage (bypasses chunk builder).
async fn insert_chunk(storage: &InMemoryStorage, chunk: Chunk) {
    storage.save_next_chunk(chunk).await.unwrap();
}

#[tokio::test]
async fn cleanup_noop_when_consistent() {
    let storage = InMemoryStorage::new_empty();
    seed_genesis_batch(&storage, test_block(0)).await;

    // Batch 1: blocks 1-3.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(3),
        3,
        vec![test_hash(1), test_hash(2)],
    )
    .unwrap();
    storage.save_next_batch(batch1).await.unwrap();

    // Chunk 0 ends at batch 1's boundary (last_block = 3).
    let chunk = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    insert_chunk(&storage, chunk).await;

    cleanup_orphaned_chunks(&storage, &storage)
        .await
        .expect("cleanup failed");

    assert!(
        storage.get_latest_chunk().await.unwrap().is_some(),
        "chunk should survive cleanup"
    );
}

#[tokio::test]
async fn cleanup_reverts_when_batch_missing() {
    let storage = InMemoryStorage::new_empty();
    seed_genesis_batch(&storage, test_block(0)).await;

    // Batch 1 exists.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(3),
        3,
        vec![test_hash(1), test_hash(2)],
    )
    .unwrap();
    storage.save_next_batch(batch1).await.unwrap();

    // Chunk 0 at batch 1 boundary (valid).
    let c0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    insert_chunk(&storage, c0).await;

    // Chunk 1 claims batch_idx=2, but batch 2 doesn't exist.
    let c1 = Chunk::new(1, test_hash(3), test_hash(5), 5, 2, vec![test_hash(4)]);
    insert_chunk(&storage, c1).await;

    cleanup_orphaned_chunks(&storage, &storage)
        .await
        .expect("cleanup failed");

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 0, "chunk 1 should be reverted");
}

#[tokio::test]
async fn cleanup_reverts_mid_batch_chunks() {
    let storage = InMemoryStorage::new_empty();
    seed_genesis_batch(&storage, test_block(0)).await;

    // Batch 1: blocks 1-6.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(6),
        6,
        vec![
            test_hash(1),
            test_hash(2),
            test_hash(3),
            test_hash(4),
            test_hash(5),
        ],
    )
    .unwrap();
    storage.save_next_batch(batch1).await.unwrap();

    // Chunk 0: blocks 1-3. Ends at block 3, not at batch 1's boundary (block 6).
    // Simulates a crash mid-batch after sealing one chunk.
    let c0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    insert_chunk(&storage, c0).await;

    cleanup_orphaned_chunks(&storage, &storage)
        .await
        .expect("cleanup failed");

    assert!(
        storage.get_latest_chunk().await.unwrap().is_none(),
        "mid-batch chunk should be reverted"
    );
}

// ---------------------------------------------------------------------------
// Backfill enqueue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backfill_enqueues_unchunked_batches() {
    use alpen_ee_common::{exec_block_storage_test_fns::create_exec_block, MockExecBlockStorage};

    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    seed_genesis_batch(&storage, genesis).await;

    // Batch 1: blocks 1-3.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(3),
        3,
        vec![test_hash(1), test_hash(2)],
    )
    .unwrap();
    storage.save_next_batch(batch1.clone()).await.unwrap();

    // Mock block storage: chain is genesis(0) <- 1 <- 2 <- 3.
    let mut mock_blocks = MockExecBlockStorage::new();
    mock_blocks.expect_get_exec_block().returning(|hash| {
        let n = hash.as_ref()[31];
        if n == 0 {
            Ok(Some(create_exec_block(0, Hash::zero(), test_hash(0), 0)))
        } else {
            Ok(Some(create_exec_block(
                n as u64,
                test_hash(n - 1),
                test_hash(n),
                n as u64,
            )))
        }
    });

    // No chunks yet, starting from genesis, batch_idx=1.
    let mut state = new_state(genesis);

    enqueue_backfill(&mut state, &storage, &mock_blocks)
        .await
        .expect("backfill failed");

    // Should have 3 block entries + 1 batch boundary.
    let mut block_count = 0;
    let mut boundary_count = 0;
    while let Some(entry) = state.pop_pending() {
        match entry {
            PendingEntry::Block { block, batch_idx } => {
                block_count += 1;
                assert_eq!(batch_idx, 1);
                assert!(block.blocknum() >= 1 && block.blocknum() <= 3);
            }
            PendingEntry::BatchBoundary(id) => {
                boundary_count += 1;
                assert_eq!(id, batch1.id());
            }
        }
    }
    assert_eq!(block_count, 3);
    assert_eq!(boundary_count, 1);
}

#[tokio::test]
async fn backfill_noop_when_caught_up() {
    use alpen_ee_common::MockExecBlockStorage;

    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    seed_genesis_batch(&storage, genesis).await;

    let mut state = new_state(genesis);
    let mock_blocks = MockExecBlockStorage::new(); // no calls expected

    enqueue_backfill(&mut state, &storage, &mock_blocks)
        .await
        .expect("backfill failed");

    assert!(!state.has_pending());
}

// ---------------------------------------------------------------------------
// Reorg → resume round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reorg_then_resume_processing() {
    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    let policy = FixedBlockCountSealing::new(3);
    seed_genesis_batch(&storage, genesis).await;
    let block_storage = mock_block_storage();

    let mut state = new_state(genesis);

    // Batch 1: blocks 1-7 → chunks [1-3] idx=0, [4-6] idx=1. Block 7 in accumulator.
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            block(7, 1),
        ],
    )
    .await;

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 1);

    // Reorg back to genesis. Genesis batch survives and unsealed chunks are discarded.
    handle_reorg(&mut state, &storage, &storage, &block_storage, genesis, 0)
        .await
        .expect("handle_reorg failed");

    assert!(
        storage.get_latest_chunk().await.unwrap().is_none(),
        "unsealed chunks should be reverted"
    );
    assert_eq!(state.prev_chunk_end(), genesis);
    assert_eq!(state.current_batch_idx(), 1);

    // Resume: feed new blocks 1-7 in batch 1 on the new fork.
    // Accumulator fills [1,2,3], block 4 triggers seal → chunk [1-3].
    process_entries(
        &mut state,
        &storage,
        &policy,
        &[
            block(1, 1),
            block(2, 1),
            block(3, 1),
            block(4, 1),
            block(5, 1),
            block(6, 1),
            block(7, 1),
        ],
    )
    .await;

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 1, "new chunk should be created after reorg");
    assert_eq!(latest.prev_block(), test_hash(3));
    assert_eq!(latest.last_block(), test_hash(6));
    assert_eq!(latest.batch_idx(), 1);

    // Block 7 should be in the accumulator.
    assert_eq!(state.accumulator().block_count(), 1);
}

// ---------------------------------------------------------------------------
// Repair batch linkage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repair_linkage_noop_when_already_linked() {
    let storage = InMemoryStorage::new_empty();
    seed_genesis_batch(&storage, test_block(0)).await;

    // Batch 1: blocks 1-3.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(3),
        3,
        vec![test_hash(1), test_hash(2)],
    )
    .unwrap();
    let batch1_id = batch1.id();
    storage.save_next_batch(batch1).await.unwrap();

    // Chunk 0 at batch boundary.
    let c0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    let c0_id = c0.id();
    insert_chunk(&storage, c0).await;

    // Pre-link the batch.
    storage
        .set_batch_chunks(batch1_id, vec![c0_id])
        .await
        .unwrap();

    repair_batch_linkage(&storage, &storage)
        .await
        .expect("repair failed");

    // Linkage unchanged.
    let linked = storage.get_batch_chunks(batch1_id).await.unwrap().unwrap();
    assert_eq!(linked, vec![c0_id]);
}

#[tokio::test]
async fn repair_linkage_reconstructs_missing_link() {
    let storage = InMemoryStorage::new_empty();
    seed_genesis_batch(&storage, test_block(0)).await;

    // Batch 1: blocks 1-6.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(6),
        6,
        vec![
            test_hash(1),
            test_hash(2),
            test_hash(3),
            test_hash(4),
            test_hash(5),
        ],
    )
    .unwrap();
    let batch1_id = batch1.id();
    storage.save_next_batch(batch1).await.unwrap();

    // Two chunks for batch 1, both at valid positions.
    // Chunk 0: blocks 1-3. Chunk 1: blocks 4-6 (at batch boundary).
    let c0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    let c1 = Chunk::new(
        1,
        test_hash(3),
        test_hash(6),
        6,
        1,
        vec![test_hash(4), test_hash(5)],
    );
    let c0_id = c0.id();
    let c1_id = c1.id();
    insert_chunk(&storage, c0).await;
    insert_chunk(&storage, c1).await;

    // No linkage — simulates crash before boundary was processed.
    assert!(storage.get_batch_chunks(batch1_id).await.unwrap().is_none());

    repair_batch_linkage(&storage, &storage)
        .await
        .expect("repair failed");

    let linked = storage.get_batch_chunks(batch1_id).await.unwrap().unwrap();
    assert_eq!(linked, vec![c0_id, c1_id], "both chunks should be linked");
}

// ---------------------------------------------------------------------------
// Full startup sequence: cleanup → repair → init → backfill
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_startup_sequence() {
    use alpen_ee_common::{exec_block_storage_test_fns::create_exec_block, MockExecBlockStorage};

    let storage = InMemoryStorage::new_empty();
    let genesis = test_block(0);
    seed_genesis_batch(&storage, genesis).await;

    // Batch 1: blocks 1-3. Batch 2: blocks 4-6.
    let batch1 = Batch::new(
        1,
        test_hash(0),
        test_hash(3),
        3,
        vec![test_hash(1), test_hash(2)],
    )
    .unwrap();
    let batch1_id = batch1.id();
    let batch2 = Batch::new(
        2,
        test_hash(3),
        test_hash(6),
        6,
        vec![test_hash(4), test_hash(5)],
    )
    .unwrap();
    storage.save_next_batch(batch1).await.unwrap();
    storage.save_next_batch(batch2).await.unwrap();

    // Simulate state after a crash:
    // - Chunk 0 [1-3] at batch 1 boundary (valid, linked).
    // - Chunk 1 [4-5] mid-batch 2 (will be reverted by cleanup).
    let c0 = Chunk::new(
        0,
        test_hash(0),
        test_hash(3),
        3,
        1,
        vec![test_hash(1), test_hash(2)],
    );
    let c0_id = c0.id();
    insert_chunk(&storage, c0).await;
    storage
        .set_batch_chunks(batch1_id, vec![c0_id])
        .await
        .unwrap();

    let c1 = Chunk::new(1, test_hash(3), test_hash(5), 5, 2, vec![test_hash(4)]);
    insert_chunk(&storage, c1).await;
    // batch 2 NOT linked — crash happened mid-batch.

    // Step 1: cleanup — reverts chunk 1 (mid-batch).
    cleanup_orphaned_chunks(&storage, &storage)
        .await
        .expect("cleanup failed");

    let (latest, _) = storage.get_latest_chunk().await.unwrap().unwrap();
    assert_eq!(latest.idx(), 0, "mid-batch chunk 1 reverted");

    // Step 2: repair — batch 1 is already linked, no-op.
    repair_batch_linkage(&storage, &storage)
        .await
        .expect("repair failed");

    // Step 3: init.
    let mut state: ChunkBuilderState<BlockCountPolicy> =
        init_chunk_builder_state(&storage, genesis)
            .await
            .expect("init failed");

    assert_eq!(state.next_chunk_idx(), 1);
    assert_eq!(state.prev_chunk_end(), test_block(3));
    assert_eq!(state.current_batch_idx(), 2, "resume at batch 2");

    // Step 4: backfill — batch 2 needs to be chunked.
    let mut mock_blocks = MockExecBlockStorage::new();
    mock_blocks.expect_get_exec_block().returning(|hash| {
        let n = hash.as_ref()[31];
        if n == 0 {
            Ok(Some(create_exec_block(0, Hash::zero(), test_hash(0), 0)))
        } else {
            Ok(Some(create_exec_block(
                n as u64,
                test_hash(n - 1),
                test_hash(n),
                n as u64,
            )))
        }
    });

    enqueue_backfill(&mut state, &storage, &mock_blocks)
        .await
        .expect("backfill failed");

    // Should have 3 block entries (blocks 4-6) + 1 batch boundary for batch 2.
    let mut block_count = 0;
    let mut boundary_count = 0;
    while let Some(entry) = state.peek_pending().cloned() {
        state.pop_pending();
        match entry {
            PendingEntry::Block { batch_idx, .. } => {
                block_count += 1;
                assert_eq!(batch_idx, 2);
            }
            PendingEntry::BatchBoundary(_) => boundary_count += 1,
        }
    }
    assert_eq!(block_count, 3);
    assert_eq!(boundary_count, 1);
}
