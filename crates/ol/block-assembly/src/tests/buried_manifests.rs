//! Tests for the buried-manifest filter applied by
//! [`BlockAssemblyAnchorContext::fetch_asm_manifests_from`].
//!
//! The filter prevents L1 reorgs from cascading into OL by only sourcing manifests
//! with at least `l1_reorg_safe_depth` L1 confirmations.

use crate::{
    context::BlockAssemblyAnchorContext,
    test_utils::{
        BlockAssemblyContextImpl, create_test_block_assembly_context, create_test_storage,
        setup_asm_state_with_l1_manifests,
    },
};

const MAX_COUNT: u32 = 100;

async fn build_ctx(asm_tip: u32, l1_reorg_safe_depth: u32) -> BlockAssemblyContextImpl {
    let storage = create_test_storage();
    setup_asm_state_with_l1_manifests(storage.as_ref(), 1, asm_tip).await;
    let (ctx, _) = create_test_block_assembly_context(storage, l1_reorg_safe_depth);
    ctx
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_only_buried_manifests() {
    // ASM tip = 10, depth = 3 -> buried tip = 8 (height 8 has 3 confirmations).
    let ctx = build_ctx(10, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, (1..=8).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_when_tip_not_yet_buried() {
    // ASM tip = 2, depth = 3 -> buried tip saturates to 0, so nothing is buried yet.
    let ctx = build_ctx(2, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");
    assert!(manifests.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn max_count_is_binding_cap() {
    // ASM tip = 100, depth = 3 -> buried tip = 98. max_count = 4 caps before buried tip.
    let ctx = build_ctx(100, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, 4)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, vec![1, 2, 3, 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_when_start_past_buried_tip() {
    // ASM tip = 5, depth = 3 -> buried tip = 3. Asking from 4 returns empty.
    let ctx = build_ctx(5, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(4, MAX_COUNT)
        .await
        .expect("fetch should succeed");
    assert!(manifests.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn depth_zero_returns_through_tip() {
    // depth = 0 is clamped to 1; the asm tip itself has 1 confirmation, so every
    // manifest from genesis through the tip is eligible.
    let ctx = build_ctx(5, 0).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, (1..=5).collect::<Vec<_>>());
}
