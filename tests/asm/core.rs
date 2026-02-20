//! Core ASM worker integration tests
//!
//! Tests the ASM worker's ability to process Bitcoin blocks and maintain state.

#![allow(
    unused_crate_dependencies,
    reason = "test dependencies shared across test suite"
)]

use bitcoin::Network;
use harness::{test_harness::create_test_harness, worker_context::TestAsmWorkerContext};
use integration_tests::harness;
use strata_asm_worker::WorkerContext;
use strata_primitives::L1BlockId;
use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};

// ============================================================================
// Worker Context
// ============================================================================

/// Verifies worker context initializes with correct defaults.
#[tokio::test(flavor = "multi_thread")]
async fn test_worker_context_initialization() {
    let (_bitcoind, client) = get_bitcoind_and_client();
    let context = TestAsmWorkerContext::new(client);

    assert_eq!(context.get_network().unwrap(), Network::Regtest);
    assert!(context.get_latest_asm_state().unwrap().is_none());
}

/// Verifies blocks are fetched from regtest and cached.
#[tokio::test(flavor = "multi_thread")]
async fn test_block_fetching_and_caching() {
    let (bitcoind, client) = get_bitcoind_and_client();
    let context = TestAsmWorkerContext::new(client);

    // Mine 5 blocks
    let block_hashes = mine_blocks(&bitcoind, context.client.as_ref(), 5, None)
        .await
        .expect("Failed to mine blocks");

    // Fetch each block through the context
    for block_hash in block_hashes.iter() {
        let block_id = L1BlockId::from(*block_hash);
        context
            .get_l1_block(&block_id)
            .expect("Failed to get block");
    }

    // Verify blocks are cached
    assert_eq!(context.block_cache.lock().unwrap().len(), 5);

    // Fetch again - should come from cache
    let block_id = L1BlockId::from(block_hashes[0]);
    let block = context
        .get_l1_block(&block_id)
        .expect("Failed to get cached block");
    assert_eq!(block.block_hash(), block_hashes[0]);
}

// ============================================================================
// Block Processing
// ============================================================================

/// Verifies ASM worker processes a single mined block.
#[tokio::test(flavor = "multi_thread")]
async fn test_single_block_processing() {
    let harness = create_test_harness()
        .await
        .expect("Failed to create test harness");

    harness
        .mine_block(None)
        .await
        .expect("Failed to mine block");

    let tip_height = harness
        .get_chain_tip()
        .await
        .expect("Failed to get chain tip");
    assert_eq!(tip_height, harness.genesis_height + 1);
}

/// Verifies ASM worker processes multiple mined blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_block_processing() {
    let harness = create_test_harness()
        .await
        .expect("Failed to create test harness");
    let (l1, state) = harness.get_latest_asm_state().unwrap().unwrap();
    assert_eq!(l1, state.state().chain_view.pow_state.last_verified_block);
    assert_eq!(
        l1.height() as u64,
        state
            .state()
            .chain_view
            .history_accumulator
            .last_inserted_height()
    );

    let block_hashes = harness.mine_blocks(3).await.expect("Failed to mine blocks");
    assert_eq!(block_hashes.len(), 3);

    let tip_height = harness
        .get_chain_tip()
        .await
        .expect("Failed to get chain tip");
    assert_eq!(tip_height, harness.genesis_height + 3);
    assert_eq!(l1, state.state().chain_view.pow_state.last_verified_block);
    assert_eq!(
        l1.height() as u64,
        state
            .state()
            .chain_view
            .history_accumulator
            .last_inserted_height()
    );
}

// ============================================================================
// MMR Integrity
// ============================================================================
//
// The ASM maintains two MMR representations of manifest hashes:
//
// **Internal (proven) MMR** — `CompactMmr64` inside `AnchorState.chain_view.history_accumulator`.
//   - Lives inside the ASM state that gets proven in the ZKVM.
//   - Compact representation: stores only peaks, not all leaves. Keeps the proven state small.
//   - Can *verify* inclusion proofs but cannot *generate* them.
//   - Updated by the STF during `compute_asm_transition`.
//   - Starts empty at genesis with `offset = genesis_height + 1`, so its index 0 is the first
//     post-genesis block.
//
// **External (full) MMR** — the worker-side database managed by `WorkerContext`.
//   - Lives outside the proven state, in the worker's persistent storage.
//   - Full tree: stores all leaves and intermediate nodes.
//   - Can *generate* inclusion proofs for any leaf via `generate_mmr_proof`.
//   - Populated by the ASM worker after each STF execution.
//
// **How they interact during checkpoint verification:**
//   1. A checkpoint tx on L1 references a range of L1 block heights.
//   2. `AuxDataResolver` uses the external MMR to generate inclusion proofs for the manifest hashes
//      at those heights.
//   3. These proofs are passed as auxiliary data into the STF.
//   4. Inside the STF, the checkpoint subprotocol verifies those proofs against the internal
//      compact MMR.
//
// The two MMRs must have identical leaves at identical indices. If the
// external MMR included the genesis manifest (which the internal does not),
// all external indices would be shifted by 1, and every proof generated
// from the external MMR would fail verification against the internal one.

/// Verifies the external (full) MMR stays index-aligned with the internal
/// (proven compact) MMR after block processing.
///
/// Mines blocks in multiple rounds and checks alignment after each round to
/// verify the invariant holds incrementally, not just at the end.
#[tokio::test(flavor = "multi_thread")]
async fn test_proven_and_external_mmr_index_alignment() {
    let harness = create_test_harness()
        .await
        .expect("Failed to create test harness");

    let genesis_height = harness.genesis_height;

    // After genesis processing, the external MMR must be empty — genesis
    // manifest is stored for L1 data consumers but NOT appended to the MMR.
    // The internal (proven) compact MMR also starts empty at this point.
    assert_eq!(
        harness.get_mmr_leaf_count(),
        0,
        "external MMR should be empty after genesis (genesis manifest must not be a leaf)"
    );

    // Mine blocks in multiple rounds of increasing size to exercise the MMR
    // across different tree shapes (powers of two, odd counts, etc.).
    // The compact MMR's internal peak structure changes at each power-of-two
    // boundary, so we want to cross several of them.
    let rounds: &[usize] = &[1, 3, 4, 8, 16];
    let mut total_blocks_mined: usize = 0;

    for (round, &count) in rounds.iter().enumerate() {
        let block_hashes = harness
            .mine_blocks(count)
            .await
            .unwrap_or_else(|e| panic!("round {round}: failed to mine {count} blocks: {e}"));
        assert_eq!(block_hashes.len(), count);
        total_blocks_mined += count;

        // -- Proven (internal) compact MMR --
        let (_commitment, latest_state) = harness
            .get_latest_asm_state()
            .unwrap_or_else(|e| panic!("round {round}: failed to get ASM state: {e}"))
            .unwrap_or_else(|| panic!("round {round}: ASM state should exist"));

        let proven_accumulator = &latest_state.state().chain_view.history_accumulator;
        let proven_offset = proven_accumulator.offset();
        let proven_tip_height = proven_accumulator.last_inserted_height();
        let proven_leaf_count = proven_tip_height - proven_offset + 1;

        assert_eq!(
            proven_offset,
            genesis_height + 1,
            "round {round}: proven MMR offset should be genesis_height + 1"
        );
        assert_eq!(
            proven_tip_height,
            genesis_height + total_blocks_mined as u64,
            "round {round}: proven MMR tip should be genesis + {total_blocks_mined}"
        );

        // -- External (full) MMR --
        let external_leaf_count = harness.get_mmr_leaf_count();

        // Core invariant: both MMRs must have the same number of leaves.
        // If genesis were erroneously in the external MMR, this would be
        // total_blocks_mined + 1 (off-by-one), breaking proof alignment.
        assert_eq!(
            proven_leaf_count as usize, external_leaf_count,
            "round {round}: proven and external MMR leaf counts must match \
             (both should be {total_blocks_mined})"
        );
    }

    // -- Leaf hash integrity over all leaves --
    // Verify every external MMR leaf matches its corresponding manifest hash.
    // This ensures the external MMR stores the exact same data the proven MMR
    // committed to, across all 32 leaves.
    let external_leaves = harness.get_mmr_leaves();
    let stored_manifests = harness.get_stored_manifests();
    let proven_offset = genesis_height + 1;

    assert_eq!(
        external_leaves.len(),
        total_blocks_mined,
        "final external MMR should have exactly {total_blocks_mined} leaves"
    );

    for (mmr_index, external_leaf_hash) in external_leaves.iter().enumerate() {
        let block_height = proven_offset + mmr_index as u64;
        let manifest = stored_manifests
            .iter()
            .find(|m| m.height() == block_height)
            .unwrap_or_else(|| panic!("no stored manifest for height {block_height}"));

        let proven_leaf_hash: [u8; 32] = manifest.compute_hash();
        assert_eq!(
            *external_leaf_hash, proven_leaf_hash,
            "leaf hash mismatch at MMR index {mmr_index} (L1 height {block_height}): \
             external MMR disagrees with manifest compute_hash()"
        );
    }

    // Verify no external leaf maps to the genesis height — genesis must
    // never appear as an MMR leaf.
    assert!(
        !stored_manifests
            .iter()
            .filter(|m| m.height() == genesis_height)
            .any(|genesis_mf| {
                let genesis_hash: [u8; 32] = genesis_mf.compute_hash();
                external_leaves.contains(&genesis_hash)
            }),
        "genesis manifest hash must not appear in the external MMR leaves"
    );
}
